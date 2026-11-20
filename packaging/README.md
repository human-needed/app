# Packages: winget and Homebrew

`python packaging/make.py v1.0.0` writes the package files for a published release into
`packaging/out/` (not committed). They point at the release's versioned installers and their
SHA-256 from `SHA256SUMS.txt`.

## winget (Windows)

Package `HumanNeeded.HumanNeeded`, user scope, the NSIS installer (silent: `/S`).

1. Check it: `winget validate --manifest packaging/out/winget/manifests/h/HumanNeeded/HumanNeeded/<version>`
   and `winget install --manifest <that folder>` on a test PC.
2. Submit: a pull request to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs)
   with that folder under `manifests/`, or `wingetcreate submit <folder>`.
3. Later versions: `wingetcreate update HumanNeeded.HumanNeeded --version <v> --urls <installer url> --submit`.

Users then run `winget install HumanNeeded.HumanNeeded`. The app updates itself after that.

## Homebrew (macOS)

The official `homebrew/cask` no longer takes apps that fail Gatekeeper, and ours is unsigned
until code signing (D-039). So the cask goes in our own tap, the repository
`human-needed/homebrew-tap`, as `Casks/humanneeded.rb`. Users then run:

```bash
brew install --cask human-needed/tap/humanneeded
```

Check it first: `brew audit --cask --new humanneeded` and `brew install --cask ./humanneeded.rb`.
`brew uninstall --zap` removes the app, its login item and its data. Once the app is signed and
notarized, the same cask can go to `homebrew/cask`.
