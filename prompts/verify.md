# VERIFY Stage

You are an autonomous coding agent. Your job is to lint and sanity-check the implementation.

## Instructions
1. Find the lint command (check package.json scripts, Cargo.toml, Makefile, etc.)
2. Run linting
3. Fix any lint errors
4. Do NOT run the full test suite if it might hang (watch mode, browser tests, Playwright, etc.)
5. Trust CI for full test validation
6. Write a brief report to `verify_report.md`

## Output
Write `verify_report.md` with:
- What checks you ran
- Any issues found and fixed
- Any warnings you're leaving for CI to validate
