# Security

Wrec records the screen, so treat any vulnerability that captures content
without consent, escapes the daemon socket boundary, or tampers with recording
output as high severity.

## Reporting

Open a [GitHub issue](https://github.com/shivamhwp/wrec/issues/new) and
include:

- what the issue is and where (app, CLI, daemon, capture engine, installer)
- steps to reproduce
- the impact you believe it has

You will get a response within a week.

## Scope notes

- The daemon listens on a unix socket at `~/.wrec/wrec.sock` and trusts local
  processes running as the same user. Anything that lets a *different* user or
  a sandboxed process drive recordings is a bug.
- Release artifacts are not notarized; verify downloads against the checksums
  in the Homebrew tap when installing manually.
