from __future__ import annotations

import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from datetime import date
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
            "overall_ranking": ["accepted_pr", "A", "B"],
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

    def example_discovery_candidate(self, *, kind_hint: str = "bug") -> dict:
        return {
            "repo": {
                "name": "example/project",
                "url": "https://github.com/example/project",
                "git_url": "https://github.com/example/project.git",
                "description": "Example project",
                "language": "TypeScript",
                "license": "MIT",
                "stars": 5000,
                "size_kb": 12000,
            },
            "issue": {
                "number": 42,
                "url": "https://github.com/example/project/issues/42",
                "title": "Fix example behavior",
                "author": "reporter",
                "created_at": "2026-01-01T00:00:00Z",
                "closed_at": "2026-01-03T00:00:00Z",
                "labels": ["bug"],
                "body_excerpt": "Issue body",
            },
            "pr": {
                "number": 99,
                "url": "https://github.com/example/project/pull/99",
                "title": "Fix example behavior",
                "author": "contributor",
                "merged_at": "2026-01-02T00:00:00Z",
                "merge_commit": "deadbeef",
                "pre_fix_commit": "abc123",
                "changed_files": 3,
                "additions": 25,
                "deletions": 7,
            },
            "kind_hint": kind_hint,
        }

    def example_discovery_selection(self) -> dict:
        return {
            "summary": "Good mix of self-contained tasks.",
            "candidates": [
                {
                    "repo_name": "example/project",
                    "issue_number": 42,
                    "pr_number": 99,
                    "kind": "bug",
                    "fit_score": 4,
                    "rationale": "Touches existing code paths without obvious solution leakage.",
                    "prompt_title": "Fix example behavior",
                    "prompt_body": "Investigate and fix the reported behavior in the repo.",
                    "evaluation_focus": ["Reuse existing code.", "Keep tests aligned."],
                }
            ],
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

    def test_build_prompt_forbids_remote_git_commands(self) -> None:
        prompt = benchmark.build_prompt(self.task, "control")

        self.assertIn("Do not run `git fetch`, `git pull`, `git checkout`", prompt)

    def test_build_discovery_shortlist_preserves_candidate_metadata(self) -> None:
        shortlist = benchmark.build_discovery_shortlist(
            self.example_discovery_selection(),
            [self.example_discovery_candidate()],
            agent="codex",
            model="gpt-5",
            generated_at="2026-03-10T12:00:00Z",
            pr_cutoff=date(2025, 9, 10),
            search_params={"repo_limit": 1},
        )

        self.assertEqual(shortlist["agent"], "codex")
        self.assertEqual(shortlist["candidates"][0]["ground_truth"]["pr_number"], 99)
        self.assertEqual(shortlist["candidates"][0]["prompt"]["title"], "Fix example behavior")

    def test_parse_judge_output_unwraps_discovery_structured_output(self) -> None:
        wrapped = json.dumps(
            {
                "type": "result",
                "structured_output": self.example_discovery_selection(),
            }
        )

        self.assertEqual(benchmark.parse_judge_output(wrapped), self.example_discovery_selection())

    def test_cmd_discover_writes_shortlist_artifacts(self) -> None:
        tasks = {"demo-task": self.task}
        raw_candidates = [self.example_discovery_candidate()]
        selection = self.example_discovery_selection()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            stdout = io.StringIO()
            with (
                mock.patch.object(benchmark, "collect_discovery_pool", return_value=raw_candidates),
                mock.patch.object(benchmark, "run_discovery_agent", return_value=selection),
                mock.patch("sys.stdout", stdout),
            ):
                exit_code = benchmark.cmd_discover(
                    tasks,
                    root,
                    agent="codex",
                    model="gpt-5",
                    pr_cutoff=date(2025, 9, 10),
                    repo_limit=1,
                    prs_per_repo=1,
                    pool_size=1,
                    candidate_count=1,
                    min_stars=100,
                    min_size_kb=100,
                )

            self.assertEqual(exit_code, 0)
            discovery_root = root / "discovery"
            runs = [path for path in discovery_root.iterdir() if path.is_dir()]
            self.assertEqual(len(runs), 1)
            self.assertTrue((runs[0] / "report.md").exists())
            self.assertTrue((runs[0] / "shortlist.json").exists())
            self.assertIn("discovered 1 candidates", stdout.getvalue())

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
        self.assertIn("\n## Task Metadata\n", report)
        self.assertIn("\n## Blind Verdict Summary\n", report)
        self.assertIn("Better matches the accepted PR: `tg`", report)
        self.assertIn("Better overall (`control` vs `tg`): `tie`", report)
        self.assertIn("Best of all three: `accepted_pr`", report)
        self.assertIn("Overall ranking: `accepted_pr` > `tg` > `control`", report)
        self.assertIn("## Blind Mapping", report)

    def test_report_markdown_falls_back_when_older_judgment_lacks_ranking(self) -> None:
        judgment = self.example_judgment()
        judgment.pop("overall_ranking")
        report = benchmark.build_report_markdown(
            task=self.task,
            evaluated_agent="codex",
            judge_agent="claude",
            eval_id="20260310T000000Z",
            judgment=judgment,
            blind_manifest=self.example_blind_manifest(),
            publish_meta=None,
        )

        self.assertIn("Overall ranking: `accepted_pr` > `tg` > `control`", report)

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
            self.assertIn("Best of all three", matrix)
            self.assertIn("How to read this table", matrix)

    def test_report_all_skips_unjudged_and_missing_runs(self) -> None:
        other_task = {
            **self.task,
            "id": "other-task",
            "issue": {
                "number": 43,
                "url": "https://github.com/example/project/issues/43",
                "title": "Other issue",
            },
            "ground_truth": {
                **self.task["ground_truth"],
                "pr_number": 100,
                "pr_url": "https://github.com/example/project/pull/100",
            },
        }
        missing_task = {
            **self.task,
            "id": "missing-task",
            "issue": {
                "number": 44,
                "url": "https://github.com/example/project/issues/44",
                "title": "Missing issue",
            },
            "ground_truth": {
                **self.task["ground_truth"],
                "pr_number": 101,
                "pr_url": "https://github.com/example/project/pull/101",
            },
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspaces"
            judged_dir = benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T000000Z")
            judged_dir.mkdir(parents=True, exist_ok=True)
            benchmark.write_json(judged_dir / "judgment.json", self.example_judgment() | {"judge_agent": "claude"})
            benchmark.write_json(judged_dir / "blind_manifest.json", self.example_blind_manifest())
            benchmark.write_json(judged_dir / "publish.json", {"published": False})

            unjudged_dir = benchmark.evaluation_dir(root, "other-task", "codex", "20260310T010000Z")
            unjudged_dir.mkdir(parents=True, exist_ok=True)
            benchmark.write_json(unjudged_dir / "blind_manifest.json", self.example_blind_manifest() | {"task_id": "other-task"})

            tasks = {
                "demo-task": self.task,
                "other-task": other_task,
                "missing-task": missing_task,
            }
            stdout = io.StringIO()
            with mock.patch("sys.stdout", stdout):
                exit_code = benchmark.cmd_report_all(tasks, [], root, evaluated_agent="codex")

            self.assertEqual(exit_code, 0)
            output = stdout.getvalue()
            self.assertIn("included runs:", output)
            self.assertIn("demo-task (codex 20260310T000000Z)", output)
            self.assertIn("skipped runs:", output)
            self.assertIn("other-task (codex 20260310T010000Z): not judged", output)
            self.assertIn("missing-task (codex): no runs found", output)
            matrix = (root.parent / "reports" / "matrix.md").read_text()
            self.assertIn("| demo-task | codex | claude |", matrix)
            self.assertNotIn("| other-task |", matrix)
            self.assertIn("higher 1-5 scores are better", matrix)
            self.assertIn("accepted_pr", matrix)

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

    def test_describe_task_runs_handles_missing_prepared_and_evaluated_states(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            self.assertEqual(benchmark.describe_task_runs(root, "demo-task"), "-")

            benchmark.run_dir(root, "demo-task").mkdir(parents=True)
            self.assertEqual(benchmark.describe_task_runs(root, "demo-task"), "prepared")

            benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T120000Z").mkdir(parents=True)
            benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T130000Z").mkdir(parents=True)
            benchmark.evaluation_dir(root, "demo-task", "claude", "20260310T140000Z").mkdir(parents=True)
            self.assertEqual(benchmark.describe_task_runs(root, "demo-task"), "codex:2, claude:1")

    def test_collect_run_records_includes_variant_and_overall_status(self) -> None:
        tasks = {"demo-task": self.task}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            published_dir = benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T140000Z")
            published_dir.mkdir(parents=True)
            (published_dir / "control.diff").write_text("diff")
            (published_dir / "tg.diff").write_text("diff")
            benchmark.write_json(published_dir / "judgment.json", self.example_judgment())
            benchmark.write_json(
                published_dir / "publish.json",
                {
                    "published": True,
                    "branches": {
                        "control": {"url": "https://example.com/control"},
                        "tg": {"url": "https://example.com/tg"},
                    },
                },
            )

            snapshotted_dir = benchmark.evaluation_dir(root, "demo-task", "claude", "20260310T130000Z")
            snapshotted_dir.mkdir(parents=True)
            (snapshotted_dir / "control.diff").write_text("diff")

            records = benchmark.collect_run_records(tasks, root)

        self.assertEqual(
            records,
            [
                {
                    "repo": "example/project",
                    "task": "demo-task",
                    "agent": "codex",
                    "run_id": "20260310T140000Z",
                    "control": "published",
                    "tg": "published",
                    "status": "published",
                },
                {
                    "repo": "example/project",
                    "task": "demo-task",
                    "agent": "claude",
                    "run_id": "20260310T130000Z",
                    "control": "snapshotted",
                    "tg": "-",
                    "status": "snapshotted",
                },
            ],
        )

    def test_render_plain_run_list_formats_run_records(self) -> None:
        rendered = benchmark.render_plain_run_list(
            [
                {
                    "repo": "example/project",
                    "task": "demo-task",
                    "agent": "codex",
                    "run_id": "20260310T140000Z",
                    "control": "published",
                    "tg": "published",
                    "status": "published",
                }
            ]
        )

        self.assertEqual(
            rendered,
            "example/project | demo-task | codex | 20260310T140000Z | control published | tg published | published\n",
        )

    def test_parse_judge_output_unwraps_claude_structured_output(self) -> None:
        wrapped = json.dumps(
            {
                "type": "result",
                "subtype": "success",
                "result": "",
                "structured_output": self.example_judgment(),
            }
        )

        self.assertEqual(benchmark.parse_judge_output(wrapped), self.example_judgment())

    def test_build_judge_prompt_references_blind_artifacts_for_three_way_comparison(self) -> None:
        judge_input = {
            "task_id": "demo-task",
            "evaluated_agent": "codex",
            "eval_id": "20260310T140000Z",
            "repo": {"name": "example/project"},
            "issue": {"number": 42, "title": "Issue title"},
            "prompt": {"title": "Prompt title", "body": "Prompt body"},
            "evaluation_focus": ["Reuse code"],
            "implementations": {
                "A": {
                    "diff_path": "judge_workspace/A.diff",
                    "repo_path": "judge_workspace/A_repo",
                    "files": {"files": []},
                },
                "B": {
                    "diff_path": "judge_workspace/B.diff",
                    "repo_path": "judge_workspace/B_repo",
                    "files": {"files": []},
                },
            },
            "ground_truth": {
                "diff_path": "judge_workspace/accepted_pr.diff",
                "repo_path": "judge_workspace/accepted_pr_repo",
                "files": {"files": []},
            },
        }

        prompt = benchmark.build_judge_prompt(judge_input)

        self.assertIn("three-way comparison", prompt)
        self.assertIn("parent -> Implementation A diff", prompt)
        self.assertIn("judge_workspace/A.diff", prompt)
        self.assertIn("judge_workspace/B_repo", prompt)
        self.assertIn("overall technical merits", prompt)

    def test_snapshot_worktree_commit_excludes_harness_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            subprocess.run(["git", "init"], cwd=repo, check=True, capture_output=True, text=True)
            subprocess.run(["git", "config", "user.name", "Benchmark Tests"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "benchmark-tests@example.com"], cwd=repo, check=True)
            (repo / "tracked.txt").write_text("before\n")
            subprocess.run(["git", "add", "tracked.txt"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "init"], cwd=repo, check=True, capture_output=True, text=True)
            base_commit = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=repo,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()

            (repo / "tracked.txt").write_text("after\n")
            (repo / ".codex" / "skills").mkdir(parents=True)
            (repo / ".codex" / "skills" / "tracegrep.md").write_text("skill\n")

            snapshot = benchmark.snapshot_worktree_commit(repo, base_commit, "snapshot")
            diff_paths = subprocess.run(
                ["git", "diff", "--name-only", base_commit, snapshot],
                cwd=repo,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.splitlines()

        self.assertEqual(diff_paths, ["tracked.txt"])

    def test_render_task_table_falls_back_without_rich(self) -> None:
        tasks = {"demo-task": self.task}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T120000Z").mkdir(parents=True)
            rendered = benchmark.render_plain_task_list(tasks, root)

        self.assertEqual(
            rendered,
            "demo-task: example/project | Add example feature | issue #42 | runs codex:1\n",
        )

    def test_cmd_list_prints_rich_table_when_available(self) -> None:
        tasks = {"demo-task": self.task}

        class FakeTable:
            def __init__(self, **kwargs: object) -> None:
                self.kwargs = kwargs
                self.columns: list[tuple[str, dict[str, object]]] = []
                self.rows: list[tuple[str, ...]] = []

            def add_column(self, name: str, **kwargs: object) -> None:
                self.columns.append((name, kwargs))

            def add_row(self, *values: str) -> None:
                self.rows.append(values)

        class FakeConsole:
            instances: list["FakeConsole"] = []

            def __init__(self, **kwargs: object) -> None:
                self.kwargs = kwargs
                self.print_calls: list[FakeTable] = []
                FakeConsole.instances.append(self)

            def print(self, table: FakeTable) -> None:
                self.print_calls.append(table)

        fake_box = mock.Mock(SIMPLE_HEAVY="simple-heavy")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            benchmark.run_dir(root, "demo-task").mkdir(parents=True)
            with (
                mock.patch.object(benchmark, "Console", FakeConsole),
                mock.patch.object(benchmark, "Table", FakeTable),
                mock.patch.object(benchmark, "box", fake_box),
            ):
                exit_code = benchmark.cmd_list(tasks, root)

        self.assertEqual(exit_code, 0)
        console = FakeConsole.instances[0]
        self.assertEqual(console.kwargs["width"], 120)
        table = console.print_calls[0]
        self.assertEqual([name for name, _ in table.columns], ["Repo", "Issue", "Task", "Runs", "Title"])
        self.assertEqual(
            table.rows,
            [
                (
                    "example/project",
                    "[link=https://github.com/example/project/issues/42]#42[/link]",
                    "demo-task",
                    "prepared",
                    "Add example feature",
                )
            ],
        )

    def test_cmd_judge_reuses_existing_managed_eval(self) -> None:
        tasks = {"demo-task": self.task}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            eval_dir = benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T150000Z")
            eval_dir.mkdir(parents=True)
            judge_workspace = benchmark.judge_workspace_dir(eval_dir)
            (judge_workspace / "A_repo").mkdir(parents=True)
            (judge_workspace / "B_repo").mkdir(parents=True)
            (judge_workspace / "accepted_pr_repo").mkdir(parents=True)
            benchmark.write_json(eval_dir / "blind_manifest.json", self.example_blind_manifest())
            (eval_dir / "control.diff").write_text("control diff")
            (eval_dir / "tg.diff").write_text("tg diff")
            (eval_dir / "ground_truth.diff").write_text("ground truth diff")
            (eval_dir / "control_files.json").write_text('{"files":[]}')
            (eval_dir / "tg_files.json").write_text('{"files":[]}')
            (eval_dir / "ground_truth_files.json").write_text('{"files":[]}')
            (judge_workspace / "A.diff").write_text("A diff")
            (judge_workspace / "B.diff").write_text("B diff")
            (judge_workspace / "accepted_pr.diff").write_text("accepted diff")
            (judge_workspace / "A_files.json").write_text('{"files":[]}')
            (judge_workspace / "B_files.json").write_text('{"files":[]}')
            (judge_workspace / "accepted_pr_files.json").write_text('{"files":[]}')
            (eval_dir / "judge_prompt.md").write_text("stale prompt")
            with (
                mock.patch.object(benchmark, "initialize_eval_run") as init_mock,
                mock.patch.object(benchmark, "write_blind_judge_artifacts") as blind_mock,
                mock.patch.object(benchmark, "run_judge_agent", return_value=self.example_judgment()) as judge_mock,
                mock.patch.object(benchmark, "render_report_for_eval", return_value=Path("/tmp/report.md")),
            ):
                exit_code = benchmark.cmd_judge(
                    tasks,
                    "demo-task",
                    root,
                    evaluated_agent="codex",
                    judge_agent="claude",
                    judge_model=None,
                    eval_id="20260310T150000Z",
                    prepare=False,
                    force=False,
                )

            judgment = benchmark.load_json(eval_dir / "judgment.json")
            refreshed_prompt = (eval_dir / "judge_prompt.md").read_text()

        self.assertEqual(exit_code, 0)
        init_mock.assert_not_called()
        blind_mock.assert_called_once()
        self.assertEqual(judgment["judge_agent"], "claude")
        self.assertNotEqual(refreshed_prompt, "stale prompt")
        self.assertEqual(judge_mock.call_args.kwargs["cwd"], eval_dir)

    def test_cmd_runs_prints_rich_table_when_available(self) -> None:
        tasks = {"demo-task": self.task}

        class FakeTable:
            def __init__(self, **kwargs: object) -> None:
                self.kwargs = kwargs
                self.columns: list[tuple[str, dict[str, object]]] = []
                self.rows: list[tuple[str, ...]] = []

            def add_column(self, name: str, **kwargs: object) -> None:
                self.columns.append((name, kwargs))

            def add_row(self, *values: str) -> None:
                self.rows.append(values)

        class FakeConsole:
            instances: list["FakeConsole"] = []

            def __init__(self, **kwargs: object) -> None:
                self.kwargs = kwargs
                self.print_calls: list[FakeTable] = []
                FakeConsole.instances.append(self)

            def print(self, table: FakeTable) -> None:
                self.print_calls.append(table)

        fake_box = mock.Mock(SIMPLE_HEAVY="simple-heavy")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            eval_dir = benchmark.evaluation_dir(root, "demo-task", "codex", "20260310T140000Z")
            eval_dir.mkdir(parents=True)
            (eval_dir / "control.diff").write_text("diff")
            (eval_dir / "tg.diff").write_text("diff")
            benchmark.write_json(eval_dir / "judgment.json", self.example_judgment())
            with (
                mock.patch.object(benchmark, "Console", FakeConsole),
                mock.patch.object(benchmark, "Table", FakeTable),
                mock.patch.object(benchmark, "box", fake_box),
            ):
                exit_code = benchmark.cmd_runs(tasks, root)

        self.assertEqual(exit_code, 0)
        console = FakeConsole.instances[0]
        self.assertEqual(console.kwargs["width"], 140)
        table = console.print_calls[0]
        self.assertEqual(
            [name for name, _ in table.columns],
            ["Repo", "Task", "Agent", "Run ID", "Control", "TG", "Status"],
        )
        self.assertEqual(
            table.rows,
            [
                (
                    "example/project",
                    "demo-task",
                    "codex",
                    "20260310T140000Z",
                    "snapshotted",
                    "snapshotted",
                    "judged",
                )
            ],
        )

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
        self.assertNotIn("--tools", run_mock.call_args.args[0])
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

    def test_runs_help(self) -> None:
        result = self.run_help("runs")
        self.assertEqual(result.returncode, 0)
        self.assertIn("runs", result.stdout)

    def test_discover_help(self) -> None:
        result = self.run_help("discover")
        self.assertEqual(result.returncode, 0)
        self.assertIn("discover", result.stdout)
        self.assertIn("--pr-cutoff", result.stdout)

    def test_main_defaults_to_runs(self) -> None:
        tasks = {"demo-task": {"id": "demo-task"}}
        with (
            mock.patch.object(benchmark, "load_tasks", return_value=tasks),
            mock.patch.object(benchmark, "cmd_runs", return_value=0) as cmd_runs_mock,
            mock.patch.object(sys, "argv", ["benchmark.py"]),
        ):
            exit_code = benchmark.main()

        self.assertEqual(exit_code, 0)
        cmd_runs_mock.assert_called_once_with(tasks, benchmark.DEFAULT_ROOT)


if __name__ == "__main__":
    unittest.main()
