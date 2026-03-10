from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
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
            "eval_id": "20260310T000000Z",
            "label_to_condition": {"A": "tg", "B": "control"},
            "condition_to_label": {"tg": "A", "control": "B"},
            "snapshot_commits": {"tg": "feedface", "control": "deadbeef"},
        }

    def example_discovery_pool(self) -> list[dict]:
        return [
            {
                "repo": {
                    "name": "acme/alpha",
                    "url": "https://github.com/acme/alpha",
                    "git_url": "https://github.com/acme/alpha.git",
                    "description": "Large TypeScript app",
                    "language": "TypeScript",
                    "license": "MIT",
                    "stars": 12000,
                    "size_kb": 48000,
                },
                "issue": {
                    "number": 101,
                    "url": "https://github.com/acme/alpha/issues/101",
                    "title": "Add configurable workspace base path",
                    "author": "issue-author",
                    "created_at": "2026-01-01T00:00:00Z",
                    "closed_at": "2026-02-01T00:00:00Z",
                    "labels": ["enhancement"],
                    "body_excerpt": "Allow callers to configure the workspace base path.",
                },
                "pr": {
                    "number": 202,
                    "url": "https://github.com/acme/alpha/pull/202",
                    "title": "Add configurable workspace base path",
                    "author": "pr-author",
                    "merged_at": "2026-02-01T00:00:00Z",
                    "merge_commit": "deadbeef",
                    "pre_fix_commit": "cafebabe",
                    "changed_files": 7,
                    "additions": 120,
                    "deletions": 20,
                },
                "kind_hint": "feature",
            },
            {
                "repo": {
                    "name": "acme/beta",
                    "url": "https://github.com/acme/beta",
                    "git_url": "https://github.com/acme/beta.git",
                    "description": "Large Rust service",
                    "language": "Rust",
                    "license": "MIT",
                    "stars": 9500,
                    "size_kb": 36000,
                },
                "issue": {
                    "number": 303,
                    "url": "https://github.com/acme/beta/issues/303",
                    "title": "Crash when cache index is empty",
                    "author": "reporter",
                    "created_at": "2025-07-01T00:00:00Z",
                    "closed_at": "2026-01-15T00:00:00Z",
                    "labels": ["bug"],
                    "body_excerpt": "The cache loader panics on empty state.",
                },
                "pr": {
                    "number": 404,
                    "url": "https://github.com/acme/beta/pull/404",
                    "title": "Fix cache loader panic",
                    "author": "fixer",
                    "merged_at": "2026-01-15T00:00:00Z",
                    "merge_commit": "facefeed",
                    "pre_fix_commit": "beadfeed",
                    "changed_files": 3,
                    "additions": 40,
                    "deletions": 8,
                },
                "kind_hint": "bug",
            },
        ]

    def example_discovery_selection(self) -> dict:
        return {
            "summary": "Good mix of one feature and one bug in large repos.",
            "candidates": [
                {
                    "repo_name": "acme/alpha",
                    "issue_number": 101,
                    "pr_number": 202,
                    "kind": "feature",
                    "fit_score": 5,
                    "rationale": "Touches existing workspace plumbing without being trivial.",
                    "prompt_title": "Add a configurable workspace base path",
                    "prompt_body": "Add support for configuring a workspace base path and update related tests and docs.",
                    "evaluation_focus": [
                        "Did the implementation reuse the existing workspace path pipeline?",
                        "Did it update tests and docs coherently?",
                    ],
                },
                {
                    "repo_name": "acme/beta",
                    "issue_number": 303,
                    "pr_number": 404,
                    "kind": "bug",
                    "fit_score": 4,
                    "rationale": "Requires navigating the cache/index path in a non-trivial service.",
                    "prompt_title": "Fix the empty-cache loader panic",
                    "prompt_body": "Fix the cache loader so an empty cache index no longer crashes, and update tests as needed.",
                    "evaluation_focus": [
                        "Did the implementation fit the existing cache-loading path cleanly?",
                        "Did it cover the empty-cache regression in tests?",
                    ],
                },
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

    def test_validate_discovery_selection_requires_bug_feature_mix(self) -> None:
        selection = self.example_discovery_selection()
        selection["candidates"] = [selection["candidates"][0]]
        with self.assertRaises(ValueError):
            benchmark.validate_discovery_selection(
                selection,
                self.example_discovery_pool(),
                candidate_count=4,
            )

    def test_cmd_discover_writes_shortlist_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "workspaces"
            with mock.patch.object(
                benchmark,
                "collect_discovery_pool",
                return_value=self.example_discovery_pool(),
            ), mock.patch.object(
                benchmark,
                "run_discovery_agent",
                return_value=self.example_discovery_selection(),
            ):
                exit_code = benchmark.cmd_discover(
                    {"demo-task": self.task},
                    root,
                    agent="codex",
                    model="gpt-5",
                    pr_cutoff=benchmark.parse_cli_date("2025-09-10"),
                    repo_limit=6,
                    prs_per_repo=8,
                    pool_size=10,
                    candidate_count=4,
                    min_stars=2000,
                    min_size_kb=5000,
                )
            self.assertEqual(exit_code, 0)
            discovery_root = root / "discovery"
            entries = list(discovery_root.iterdir())
            self.assertEqual(len(entries), 1)
            shortlist = json.loads((entries[0] / "shortlist.json").read_text())
            self.assertEqual(shortlist["pr_cutoff"], "2025-09-10")
            self.assertEqual(len(shortlist["candidates"]), 2)
            self.assertTrue((entries[0] / "report.md").exists())

    def test_collect_discovery_pool_skips_existing_issue_pr_but_allows_same_repo(self) -> None:
        existing_task = self.task | {
            "repo": self.task["repo"] | {"name": "acme/alpha"},
            "issue": self.task["issue"] | {"number": 101},
            "ground_truth": self.task["ground_truth"] | {"pr_number": 202},
        }
        repo = {
            "name": "acme/alpha",
            "url": "https://github.com/acme/alpha",
            "git_url": "https://github.com/acme/alpha.git",
            "description": "Large TypeScript app",
            "language": "TypeScript",
            "license": "MIT",
            "stars": 12000,
            "size_kb": 48000,
        }
        repo_candidates = [
            self.example_discovery_pool()[0],
            self.example_discovery_pool()[0]
            | {
                "issue": self.example_discovery_pool()[0]["issue"] | {"number": 102},
                "pr": self.example_discovery_pool()[0]["pr"] | {"number": 203},
            },
        ]
        with mock.patch.object(benchmark, "search_candidate_repositories", return_value=[repo]), mock.patch.object(
            benchmark, "search_recent_repo_candidates", return_value=repo_candidates
        ):
            pool = benchmark.collect_discovery_pool(
                {"existing": existing_task},
                repo_limit=5,
                prs_per_repo=5,
                pool_size=5,
                min_stars=2000,
                min_size_kb=5000,
                pr_cutoff=benchmark.parse_cli_date("2025-09-10"),
            )
        self.assertEqual(len(pool), 1)
        self.assertEqual(pool[0]["repo"]["name"], "acme/alpha")
        self.assertEqual(pool[0]["issue"]["number"], 102)
        self.assertEqual(pool[0]["pr"]["number"], 203)


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
        self.assertIn("--judge-model", result.stdout)

    def test_discover_help(self) -> None:
        result = self.run_help("discover")
        self.assertEqual(result.returncode, 0)
        self.assertIn("discover", result.stdout)
        self.assertIn("--pr-cutoff", result.stdout)


if __name__ == "__main__":
    unittest.main()
