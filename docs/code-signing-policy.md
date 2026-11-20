# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

## What is signed

Only files built from this repository's source code by the project's own release
workflow are signed:

- the Windows installer (`*-setup.exe`) attached to each
  [GitHub release](https://github.com/FileManagerUltra/FileManagerUltra/releases)
- the `filemanagerultra.exe` program it installs

Builds run on GitHub-hosted runners, using
[`.github/workflows/release.yml`](../.github/workflows/release.yml), and are started
by pushing a version tag (`vX.Y.Z`). Nothing built on a developer's own computer is
signed. Third-party open-source components are signed only as part of these builds,
in the versions locked in `Cargo.lock`, and they are listed in
[THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md). No proprietary or closed-source
components are included.

The macOS disk image is signed and notarized separately with the project's own Apple
Developer ID. Linux packages are not signed.

## Team roles

| Role | Members |
|---|---|
| Committers and reviewers | People with write access to the [repository](https://github.com/FileManagerUltra/FileManagerUltra) |
| Approvers | [Owners](https://github.com/orgs/FileManagerUltra/people?query=role%3Aowner) of the FileManagerUltra GitHub organization |

Currently the project has one maintainer, Remko Silvis ([@CyberSecutor](https://github.com/CyberSecutor)),
who holds all three roles. Other organization members have read access only.

- **Committers** may change the source code. All changes go through the protected
  `main` branch.
- **Reviewers** check changes from other contributors before they are merged.
  Pull requests from outside contributors are never merged without review.
- **Approvers** approve each signing request in SignPath. A request is approved only
  for a build started by a release tag that matches the version in `Cargo.toml`.

Everyone with one of these roles uses multi-factor authentication for GitHub and
SignPath.

## Privacy policy

This program will not transfer any information to other networked systems unless
specifically requested by the user or the person installing or operating it, with
the one exception listed first below.

- **Update check (on by default).** At startup the program asks GitHub
  (`api.github.com`) for the latest release of this project. The request contains
  only the program's name and version. As with any web request, GitHub can see your
  IP address. No other data is sent, and nothing is downloaded or installed: if a
  newer version exists, the program shows a link that opens in your browser. Turn
  this off under *File → Preferences → Check for updates at startup*. *Help → Check
  for Updates* still works when it is off.
- **Connections you make.** The program contacts SMB and SFTP servers only when you
  open or connect to them.
- **Network browsing.** Opening the *Network* location looks for file servers on your
  local network (mDNS and WS-Discovery). These messages stay on your local network.
- **Remote images in Markdown previews.** These are off by default. When you turn
  them on (*File → Preferences → Load remote images in Markdown*), opening a Markdown
  file loads the images it links to from their web servers.
- **Links.** Links you click open in your web browser.

The program has no telemetry, analytics or crash reporting.
