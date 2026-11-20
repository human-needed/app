# Contributing to HumanNeeded

Thanks for helping! Bug reports, ideas and pull requests are all welcome.

## Before you start

- **Bugs and ideas:** open an [issue](https://github.com/human-needed/app/issues/new/choose) first,
  so we can agree on the fix before you spend time on it.
- **Security problems:** don't open an issue. See [SECURITY.md](SECURITY.md).
- **Small fixes** (typos, docs, an obvious one-line bug) can go straight to a pull request.

## Build and test

You need [Rust](https://rustup.rs) (stable). On Linux, Tauri also needs its
[system packages](https://v2.tauri.app/start/prerequisites/#linux) plus `libudev-dev` and
`libasound2-dev`.

```bash
cargo test --workspace
```

```bash
cargo run -p humanneeded
```

CI runs the same three checks on Windows, macOS and Linux. Run them before you push:

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

You don't need a puck: `HN_PUCK=sim` runs a simulated one inside the app, and with no puck the
app shows everything on screen. See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for the layout,
the event API and the simulator.

## Pull requests

- One change per pull request, with a short description of what and why.
- Add or update tests for behaviour you change.
- Keep the code style: `cargo fmt`, no clippy warnings, comments that say *why*.
- UI text: short, plain words; the same state names everywhere (Working, Needs you, Done, Idle).
- New source files start with `// SPDX-License-Identifier: FSL-1.1-ALv2`.
- Add a line under **Unreleased** in [CHANGELOG.md](CHANGELOG.md) for anything a user would notice.

## Sign your commits (DCO)

Every commit must carry a `Signed-off-by` line. It certifies that you wrote the change, or
otherwise have the right to submit it under this project's licence, as set out in the
[Developer Certificate of Origin 1.1](https://developercertificate.org/):

```text
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.


Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

Add the line with `git commit -s`. It uses your Git name and email:

```text
Signed-off-by: Your Name <you@example.com>
```

Forgot? `git commit --amend -s` fixes the last commit; `git rebase --signoff main` fixes them all.

## Licence

HumanNeeded is licensed under the [Functional Source License, Version 1.1, ALv2 Future
License](LICENSE) (FSL-1.1-ALv2). By contributing, you agree that your contribution is licensed
under the same terms, including its change to the Apache License 2.0 two years after release. In
the DCO above, "the open source license" means this licence.
