from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path


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
            "eval_id": "20260310T000000Z",
            "label_to_condition": {"A": "tg", "B": "control"},
            "condition_to_label": {"tg": "A", "control": "B"},
            "snapshot_commits": {"tg": "feedface", "control": "deadbeef"},
        }

    def test_blind_manifest_is_deterministic(self) -> None:
        first = benchmark.build_blind_manifest(
            task_id="demo-task",
            evaluated_agent="codex",
            eval_id="20260310T000000Z",
            snapshot_commits={"control": "a", "tg": "b"},
        )
        second = benchmark.build_blind_manifest(
            task_id="demo-task",
            evaluated_agent="codex",
            eval_id="20260310T000000Z",
            snapshot_commits={"control": "a", "tg": "b"},
        )
        self.assertEqual(first["label_to_condition"], second["label_to_condition"])

    def test_branch_names_are_opaque(self) -> None:
        branches = benchmark.branch_names("demo-task", "codex", "20260310T000000Z")
        self.assertTrue(branches["control"].startswith("benchmark/"))
        self.assertTrue(branches["control"].endswith("/control"))
        self.assertTrue(branches["tg"].endswith("/tg"))
        self.assertNotIn("demo-task", branches["control"])

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
            eval_id="20260310T000000Z",
            judgment=self.example_judgment(),
            blind_manifest=self.example_blind_manifest(),
            publish_meta=None,
        )
        self.assertIn("Publishing has not been run yet", report)
        self.assertIn("Better matches the accepted PR: `tg`", report)
        self.assertIn("## Blind Mapping", report)

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
            eval_id="20260310T000000Z",
            judgment=self.example_judgment(),
            blind_manifest=self.example_blind_manifest(),
            publish_meta=publish_meta,
        )
        self.assertIn("[control branch](https://github.com/btucker/project/tree/branch-control)", report)
        self.assertIn("[compare](https://github.com/btucker/project/compare/control...tg)", report)

    def test_render_report_and_matrix_from_fake_eval(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspaces"
            eval_dir = benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T000000Z")
            eval_dir.mkdir(parents=True, exist_ok=True)
            benchmark.write_json(eval_dir / "judgment.json", self.example_judgment() | {"judge_agent": "claude"})
            benchmark.write_json(eval_dir / "blind_manifest.json", self.example_blind_manifest())
            benchmark.write_json(eval_dir / "publish.json", {"published": False})
            report_path = benchmark.render_report_for_eval(
                task=self.task,
                evaluated_agent="codex",
                judge_agent="claude",
                root=root,
                eval_dir=eval_dir,
            )
            self.assertTrue(report_path.exists())
            tasks = {"demo-task": self.task}
            exit_code = benchmark.cmd_report_all(tasks, ["demo-task"], root, evaluated_agent="codex")
            self.assertEqual(exit_code, 0)
            self.assertTrue((root.parent / "reports" / "matrix.md").exists())
            matrix = (root.parent / "reports" / "matrix.md").read_text()
            self.assertIn("| demo-task | codex | claude |", matrix)

    def test_latest_eval_dir_requires_existing_eval(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(SystemExit):
                benchmark.latest_eval_dir(root, "missing-task", "codex")

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


class BenchmarkCliSmokeTests(unittest.TestCase):
    def run_help(self, subcommand: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "eval/benchmark.py", subcommand, "--help"],
            cwd=Path(__file__).resolve().parents[1],
            text=True,
            capture_output=True,
            check=False,
        )

    def test_judge_help(self) -> None:
        result = self.run_help("judge")
        self.assertEqual(result.returncode, 0)
        self.assertIn("judge", result.stdout)

    def test_publish_help(self) -> None:
        result = self.run_help("publish")
        self.assertEqual(result.returncode, 0)
        self.assertIn("publish", result.stdout)

    def test_report_help(self) -> None:
        result = self.run_help("report")
        self.assertEqual(result.returncode, 0)
        self.assertIn("report", result.stdout)

    def test_run_task_help(self) -> None:
        result = self.run_help("run-task")
        self.assertEqual(result.returncode, 0)
        self.assertIn("run-task", result.stdout)
        self.assertIn("--agent-model", result.stdout)
        self.assertIn("--judge-model", result.stdout)


if __name__ == "__main__":
    unittest.main()
