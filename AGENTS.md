# AGENTS.md

This file defines how AI agents should behave in this repository.

## Shell environment

- The default interactive terminal for this repo is **Nu shell (nushell)**.
- **However, when running commands, always use Bash syntax** (POSIX/GNU tooling, pipes, `&&`, globs, etc.).
- To do that safely from Nu, **invoke Bash explicitly**.

### Required rule

When you need to execute a command, run it through Bash using the `bash` keyword:

- Prefer:
  - `bash -lc '<your bash command(s)>'`

`-l` loads a login shell (consistent PATH), and `-c` runs the command string.

### Examples

Run a single command:

- `bash -lc 'ls -la'`

Run multiple commands:

- `bash -lc 'cd apps/backend && pnpm install && pnpm test'`

Use Bash-only features (globs, pipes, subshells):

- `bash -lc 'cat apps/backend/package.json | jq .name'`
- `bash -lc 'for f in apps/*/package.json; do echo "$f"; done'`

Longer scripts (recommended):

- `bash -lc 'set -euo pipefail
  cd apps/backend
  pnpm lint
  pnpm test
  '

## Output formatting for agents

- When showing commands in messages, use fenced code blocks labeled `bash`.
- When actually executing commands from Nu, still wrap them as `bash -lc '…'`.

## Don’ts

- Don’t use Nu-specific pipelines/filters (`where`, `each`, `select`, etc.) in command suggestions.
- Don’t assume the terminal understands Bash syntax unless it is executed via `bash -lc`.
- Don’t run interactive Bash sessions unless explicitly required.
