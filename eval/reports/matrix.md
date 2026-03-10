# Benchmark Matrix

| Task | Agent | Judge | Better vs PR | Better overall (control vs tg) | Best of all three | PR align control | PR align tg | Reuse control | Reuse tg | Duplication risk control | Duplication risk tg | Confidence | Control branch | TG branch | Report |
| --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| storybook-hide-toolbar-docs | codex | claude | tg | control | accepted_pr | 2 | 3 | 2 | 3 | 2 | 3 | medium | n/a | n/a | reports/storybook-hide-toolbar-docs/codex/20260310T154045Z.md |
| vscode-ghostty-external-terminal | codex | claude | tg | control | accepted_pr | 2 | 2 | 3 | 2 | 3 | 3 | high | n/a | n/a | reports/vscode-ghostty-external-terminal/codex/20260310T175223Z.md |
| eslint-no-duplicate-imports-unmergeable | codex | claude | tg | tg | accepted_pr | 2 | 4 | 3 | 4 | 2 | 4 | high | n/a | n/a | reports/eslint-no-duplicate-imports-unmergeable/codex/20260310T175219Z.md |
| pnpm-sqlite-store-index | codex | claude | tg | tg | accepted_pr | 2 | 3 | 3 | 3 | 2 | 3 | medium | n/a | n/a | reports/pnpm-sqlite-store-index/codex/20260310T175221Z.md |
| vitest-flaky-summary-reporter | codex | claude | tg | tg | accepted_pr | 2 | 2 | 3 | 3 | 4 | 4 | high | n/a | n/a | reports/vitest-flaky-summary-reporter/codex/20260310T124650Z.md |

_How to read this table: higher 1-5 scores are better. `PR align` measures closeness to the accepted PR, `Reuse` measures alignment with existing code patterns, and `Duplication risk` reflects how well the implementation avoids unnecessary duplication. `Better overall (control vs tg)` is the head-to-head winner between the two benchmark implementations, while `Best of all three` can also be `accepted_pr`._
