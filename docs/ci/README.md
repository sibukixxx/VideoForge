# CI workflow

`github-actions-ci.yml` is the Windows / macOS / Ubuntu matrix described in
design §40 (fmt, clippy, test, release build, offline smoke).

It lives here instead of `.github/workflows/` because the automated session
that authored it does not have permission to create workflow files. To enable
it:

```bash
mkdir -p .github/workflows
git mv docs/ci/github-actions-ci.yml .github/workflows/ci.yml
```
