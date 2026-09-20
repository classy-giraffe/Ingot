# Code Quality and Development Rules

Standards for code architecture, sizing, testing, and commit hygiene in Ingot.

## Sizing and Modularity

- Keep functions under 50 lines. Split orchestration functions into focused, single-responsibility helpers.
- Keep files under 500 lines. Split on domain boundaries and module depth rather than line count alone.
- Group related parameters into dataclasses or named structures to eliminate data clumps.
- Avoid middle-man delegations and raw primitive obsession where domain types add type safety.

## Test-Driven Development (TDD)

- Test at public seams only. Never assert private internals, mock plumbing, or mirror source text.
- Follow the red-green loop: write the failing test first, write minimal code to pass, then refactor.
- Run fast host-side test suites (`just test` for pytest, `just rust-test` for cargo) regularly.
- Full VM harness tests (`just harness`, `just iso`, `just ab`) test booted system invariants and should run at integration checkpoints.

## Code Style and Punctuation

- No em- or en-dashes in code, comments, documentation, or commit messages. Use single hyphens instead.
- Python code must pass `ruff check` and `ruff format` cleanly without warnings.
- Rust code must pass `cargo test` and `cargo clippy` with zero warnings.

## Commits and Git Hygiene

- Conventional Commits format required: `<type>[optional scope]: <description>`.
- Imperative mood, under 50 characters in subject line, no period at end. Body wrapped at 72 columns explaining why, not how.
- Reference issues in footers (e.g., `Closes #20`).
- Every commit ends with a sign-off trailer identifying the tool and model:
  `Signed-by: omp (<provider> <model>)`.
- Never commit private keys, build caches, or temporary test disks.
