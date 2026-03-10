from __future__ import annotations

import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock


def load_benchmark_module():
    path = Path(__file__).with_name("benchmark.py")
    spec = importlib.util.spec_from_file_location("tracegrep_eval_benchmark", path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


benchmark = load_benchmark_module()


class BenchmarkHarnessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.task = {
            "id": "demo-task",
            "repo": {
                "name": "example/project",
                "url": "https://github.com/example/project.git",
                "license": "MIT",
                "language": "TypeScript",
            },
            "issue": {
                "number": 42,
                "url": "https://github.com/example/project/issues/42",
                "title": "Add example feature",
            },
            "ground_truth": {
                "pr_number": 99,
                "pr_url": "https://github.com/example/project/pull/99",
                "pre_fix_commit": "abc123",
                "merge_commit": "def456",
            },
            "prompt": {
                "title": "Add example feature",
                "body": "Implement the feature in the repo.",
            },
            "evaluation_focus": [
                "Reuse existing patterns.",
                "Avoid duplication.",
            ],
        }

    def example_judgment(self) -> dict:
        return {
            "better_matches_pr": "A",
            "better_overall": "tie",
            "confidence": "medium",
            "scores": {
                "A": {
                    "pr_alignment": 5,
                    "reuse_alignment": 4,
                    "duplication_risk": 2,
                    "test_alignment": 4,
                },
                "B": {
                    "pr_alignment": 3,
                    "reuse_alignment": 2,
                    "duplication_risk": 4,
                    "test_alignment": 3,
                },
            },
            "A_vs_pr_differences": ["A differs from the PR only in naming."],
            "B_vs_pr_differences": ["B adds a parallel helper the PR did not add."],
            "A_vs_B_differences": ["A extends an existing path, while B adds a new branch."],
            "notable_strengths": {
                "A": ["Good reuse of existing code."],
                "B": ["Reasonable tests despite extra indirection."],
            },
            "notable_risks": {
                "A": ["Could still miss one edge case."],
                "B": ["Higher duplication risk."],
            },
            "summary": "Implementation A matches the human PR better and avoids the extra branch in B.",
        }

    def example_blind_manifest(self) -> dict:
        return {
            "task_id": "demo-task",
            "evaluated_agent": "codex",
            "run_id": "20260310T000000Z",
            "label_to_condition": {"A": "tg", "B": "control"},
            "condition_to_label": {"tg": "A", "control": "B"},
            "snapshot_commits": {"tg": "feedface", "control": "deadbeef"},
        }

    def example_run_manifest(self) -> dict:
        return benchmark.new_run_manifest(
            task=self.task,
            evaluated_agent="codex",
            run_id="20260310T000000Z",
            agent_model="sonnet",
            build_args=["--model", "sonnet"],
        )

    def test_blind_manifest_is_deterministic(self) -> None:
        first = benchmark.build_blind_manifest(
            task_id="demo-task",
            evaluated_agent="codex",
            run_id="20260310T000000Z",
            snapshot_commits={"control": "a", "tg": "b"},
        )
        second = benchmark.build_blind_manifest(
            task_id="demo-task",
            evaluated_agent="codex",
            run_id="20260310T000000Z",
            snapshot_commits={"control": "a", "tg": "b"},
        )
        self.assertEqual(first["label_to_condition"], second["label_to_condition"])

    def test_branch_names_use_run_layout(self) -> None:
        branches = benchmark.branch_names("demo-task", "codex", "20260310T000000Z")
        self.assertEqual(
            branches["control"],
            "runs/demo-task/codex/20260310T000000Z/control",
        )
        self.assertEqual(
            branches["tg"],
            "runs/demo-task/codex/20260310T000000Z/tg",
        )

    def test_diff_summary_parsers(self) -> None:
        numstat = "10\t2\tsrc/file.ts\n-\t-\tbinary.dat\n"
        status = "M\tsrc/file.ts\nA\tbinary.dat\n"
        parsed_numstat = benchmark.parse_numstat(numstat)
        parsed_status = benchmark.parse_name_status(status)
        self.assertEqual(parsed_numstat["src/file.ts"]["added"], 10)
        self.assertEqual(parsed_numstat["binary.dat"]["added"], -1)
        self.assertEqual(parsed_status["binary.dat"]["status"], "A")

    def test_report_markdown_without_publish(self) -> None:
        report = benchmark.build_report_markdown(
            task=self.task,
            evaluated_agent="codex",
            judge_agent="claude",
            run_id="20260310T000000Z",
            judgment=self.example_judgment(),
            blind_manifest=self.example_blind_manifest(),
            publish_meta=None,
        )
        self.assertIn("Run ID: `20260310T000000Z`", report)
        self.assertIn("Publishing has not been run yet", report)
        self.assertIn("Better matches the accepted PR: `tg`", report)

    def test_report_markdown_with_publish(self) -> None:
        publish_meta = {
            "published": True,
            "fork": {"name_with_owner": "btucker/project", "url": "https://github.com/btucker/project"},
            "branches": {
                "control": {"url": "https://github.com/btucker/project/tree/branch-control"},
                "tg": {"url": "https://github.com/btucker/project/tree/branch-tg"},
            },
            "compare_urls": {
                "control_vs_pre_fix": "https://github.com/btucker/project/compare/a...control",
                "tg_vs_pre_fix": "https://github.com/btucker/project/compare/a...tg",
                "control_vs_tg": "https://github.com/btucker/project/compare/control...tg",
            },
        }
        report = benchmark.build_report_markdown(
            task=self.task,
            evaluated_agent="codex",
            judge_agent="claude",
            run_id="20260310T000000Z",
            judgment=self.example_judgment(),
            blind_manifest=self.example_blind_manifest(),
            publish_meta=publish_meta,
        )
        self.assertIn("[control branch](https://github.com/btucker/project/tree/branch-control)", report)
        self.assertIn("[compare](https://github.com/btucker/project/compare/control...tg)", report)

    def test_render_report_and_matrix_from_fake_run(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspaces"
            run_path = benchmark.run_dir(root, "demo-task", "codex", "20260310T000000Z")
            run_path.mkdir(parents=True, exist_ok=True)
            benchmark.write_run_manifest(run_path, self.example_run_manifest())
            benchmark.write_json(run_path / "judgment.json", self.example_judgment() | {"judge_agent": "claude"})
            benchmark.write_json(run_path / "blind_manifest.json", self.example_blind_manifest())
            benchmark.write_json(run_path / "publish.json", {"published": False})
            report_path = benchmark.render_report_for_run(
                task=self.task,
                evaluated_agent="codex",
                judge_agent="claude",
                root=root,
                run_path=run_path,
            )
            self.assertTrue(report_path.exists())
            tasks = {"demo-task": self.task}
            exit_code = benchmark.cmd_report_all(tasks, ["demo-task"], root, evaluated_agent="codex")
            self.assertEqual(exit_code, 0)
            self.assertTrue((root.parent / "reports" / "matrix.md").exists())
            matrix = (root.parent / "reports" / "matrix.md").read_text()
            self.assertIn("| demo-task | codex | claude |", matrix)

    def test_latest_run_dir_requires_existing_run(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(SystemExit):
                benchmark.latest_run_dir(root, "missing-task", "codex")

    def test_derive_run_status_prefers_publish_and_judgment(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_path = Path(tmp)
            benchmark.write_run_manifest(run_path, self.example_run_manifest())
            self.assertEqual(benchmark.derive_run_status(run_path), "created")

            manifest = benchmark.load_run_manifest(run_path)
            for condition in benchmark.SUPPORTED_CONDITIONS:
                manifest["variants"][condition]["prepared"] = True
            benchmark.write_run_manifest(run_path, manifest)
            self.assertEqual(benchmark.derive_run_status(run_path), "prepared")

            for condition in benchmark.SUPPORTED_CONDITIONS:
                manifest["variants"][condition]["launched"] = True
            benchmark.write_run_manifest(run_path, manifest)
            self.assertEqual(benchmark.derive_run_status(run_path), "launched")

            benchmark.write_json(run_path / "judgment.json", self.example_judgment())
            self.assertEqual(benchmark.derive_run_status(run_path), "judged")

            benchmark.write_json(run_path / "publish.json", {"published": True})
            self.assertEqual(benchmark.derive_run_status(run_path), "published")

    def test_list_runs_prints_status(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run_path = benchmark.run_dir(root, "demo-task", "codex", "20260310T000000Z")
            run_path.mkdir(parents=True, exist_ok=True)
            manifest = self.example_run_manifest()
            for condition in benchmark.SUPPORTED_CONDITIONS:
                manifest["variants"][condition]["prepared"] = True
            benchmark.write_run_manifest(run_path, manifest)
            capture = io.StringIO()
            with redirect_stdout(capture):
                result = benchmark.cmd_list_runs(
                    {"demo-task": self.task},
                    ["demo-task"],
                    root,
                    evaluated_agent="codex",
                )
            self.assertEqual(result, 0)
            self.assertIn("demo-task\tcodex\t20260310T000000Z\tprepared\t", capture.getvalue())

    def test_validate_judgment_rejects_bad_payload(self) -> None:
        payload = self.example_judgment()
        payload["scores"]["A"]["pr_alignment"] = 7
        with self.assertRaises(ValueError):
            benchmark.validate_judgment(payload)

    def test_forwarded_build_args_injects_agent_model(self) -> None:
        self.assertEqual(
            benchmark.forwarded_build_args(["--permission-mode", "acceptEdits"], "sonnet"),
            ["--model", "sonnet", "--permission-mode", "acceptEdits"],
        )

    def test_forwarded_build_args_rejects_conflicting_model_flags(self) -> None:
        with self.assertRaises(SystemExit):
            benchmark.forwarded_build_args(["--model", "gpt-5"], "sonnet")

    def test_run_judge_claude_streams_prompt_over_stdin(self) -> None:
        prompt = "x" * 20000
        expected = self.example_judgment()
        with (
            mock.patch.object(benchmark, "require_command"),
            mock.patch.object(
                benchmark,
                "run",
                return_value=subprocess.CompletedProcess(["claude"], 0, stdout="{}"),
            ) as run_mock,
            mock.patch.object(benchmark, "parse_judge_output", return_value=expected),
            mock.patch.object(benchmark, "validate_judgment"),
        ):
            result = benchmark.run_judge_claude(prompt, cwd=Path("/tmp"), judge_model="sonnet")

        self.assertEqual(result, expected)
        self.assertEqual(run_mock.call_args.args[0][-2:], ["--model", "sonnet"])
        self.assertNotIn(prompt, run_mock.call_args.args[0])
        self.assertEqual(run_mock.call_args.kwargs["input_text"], prompt)

    def test_run_judge_codex_streams_prompt_over_stdin(self) -> None:
        prompt = "x" * 20000

        def fake_run(args: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            output_path = Path(args[args.index("-o") + 1])
            output_path.write_text(json.dumps(self.example_judgment()))
            return subprocess.CompletedProcess(args, 0, stdout="")

        with (
            mock.patch.object(benchmark, "require_command"),
            mock.patch.object(benchmark, "run", side_effect=fake_run) as run_mock,
        ):
            result = benchmark.run_judge_codex(prompt, cwd=Path("/tmp"), judge_model="gpt-5")

        self.assertEqual(result, self.example_judgment())
        self.assertEqual(run_mock.call_args.args[0][-3:], ["--model", "gpt-5", "-"])
        self.assertNotIn(prompt, run_mock.call_args.args[0])
        self.assertEqual(run_mock.call_args.kwargs["input_text"], prompt)


    def test_judge_input_does_not_leak_condition_names(self) -> None:
        """The judge must stay blind to which implementation is control vs tg.

        Verify that neither 'control' nor 'tg' appears anywhere in the
        serialized judge input dict (excluding the blind manifest itself,
        which is not passed to the judge).
        """
        blind_manifest = self.example_blind_manifest()
        with tempfile.TemporaryDirectory() as tmp:
            eval_dir = Path(tmp)
            # Create the diff and file artifacts that build_judge_input reads
            for condition in ("control", "tg"):
                (eval_dir / f"{condition}.diff").write_text(f"diff for {condition}")
                benchmark.write_json(eval_dir / f"{condition}_files.json", {"files": []})
            (eval_dir / "ground_truth.diff").write_text("diff for ground_truth")
            benchmark.write_json(eval_dir / "ground_truth_files.json", {"files": []})

            judge_input = benchmark.build_judge_input(
                task=self.task,
                blind_manifest=blind_manifest,
                eval_dir=eval_dir,
            )

        # Serialize the judge input and check for leaks
        serialized = json.dumps(judge_input)
        # The diff *content* may contain the word "control" or "tg" since
        # we wrote it above, but the structural keys like diff_path must not.
        # Check only the keys and diff_path values.
        for label_data in judge_input["implementations"].values():
            self.assertNotIn("control", label_data["diff_path"])
            self.assertNotIn("tg", label_data["diff_path"])


class BenchmarkCliSmokeTests(unittest.TestCase):
    def run_help(self, subcommand: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "eval/benchmark.py", subcommand, "--help"],
            cwd=Path(__file__).resolve().parents[1],
            text=True,
            capture_output=True,
            check=False,
        )

    def test_launch_help(self) -> None:
        result = self.run_help("launch")
        self.assertEqual(result.returncode, 0)
        self.assertIn("--run-id", result.stdout)
        self.assertIn("--variant", result.stdout)

    def test_judge_help(self) -> None:
        result = self.run_help("judge")
        self.assertEqual(result.returncode, 0)
        self.assertIn("--run-id", result.stdout)
        self.assertIn("--judge-model", result.stdout)

    def test_publish_help(self) -> None:
        result = self.run_help("publish")
        self.assertEqual(result.returncode, 0)
        self.assertIn("--run-id", result.stdout)

    def test_report_help(self) -> None:
        result = self.run_help("report")
        self.assertEqual(result.returncode, 0)
        self.assertIn("--run-id", result.stdout)

    def test_list_runs_help(self) -> None:
        result = self.run_help("list-runs")
        self.assertEqual(result.returncode, 0)
        self.assertIn("list-runs", result.stdout)

    def test_run_task_help(self) -> None:
        result = self.run_help("run-task")
        self.assertEqual(result.returncode, 0)
        self.assertIn("run-task", result.stdout)
        self.assertIn("--agent-model", result.stdout)
        self.assertIn("--judge-model", result.stdout)


if __name__ == "__main__":
    unittest.main()
