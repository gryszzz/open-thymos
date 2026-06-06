# Homebrew tap setup

Goal: `brew install gryszzz/tap/thymos` installs the `thymos` CLI + `thymos-server`
from the published GitHub release binaries — no Rust, no compile.

Homebrew taps are a **separate repo** named `homebrew-<name>`. Two one-time steps,
then it stays current automatically.

## 1. Create the tap repo + seed the formula (works immediately)

```bash
# create the tap repo (public)
gh repo create gryszzz/homebrew-tap --public \
  --description "Homebrew tap for OpenThymos"

# seed it with the verified v0.5.0 formula from this repo
git clone https://github.com/gryszzz/homebrew-tap.git /tmp/homebrew-tap
mkdir -p /tmp/homebrew-tap/Formula
cp packaging/homebrew/thymos.rb /tmp/homebrew-tap/Formula/thymos.rb
cd /tmp/homebrew-tap
git add Formula/thymos.rb && git commit -m "thymos v0.5.0" && git push
```

Now anyone can:

```bash
brew install gryszzz/tap/thymos
thymos doctor
```

`thymos.rb` here is pinned to **v0.5.0 with real sha256 sums**, so install works
the moment it lands in the tap — you do not need to cut a new release first.

## 2. Keep it current automatically (optional)

Add a `HOMEBREW_TAP_TOKEN` secret to the **open-thymos** repo: a PAT (or
fine-grained token) with **write access to `gryszzz/homebrew-tap`**.

```bash
gh secret set HOMEBREW_TAP_TOKEN --repo gryszzz/open-thymos
# paste the token when prompted
```

After that, the `homebrew` job in `.github/workflows/release.yml` regenerates
`Formula/thymos.rb` from the release tarballs on every `vX.Y.Z` tag. Without the
secret that job is a clean no-op, so nothing breaks until you opt in.

## Notes

- The formula installs prebuilt binaries (a "bottle"-less binary formula); it
  does not compile from source, matching `scripts/get.sh`.
- macOS Apple Silicon + Intel and Linux x86_64 are covered. Windows users use the
  `.msi` (once the desktop job ships) or `scoop`/direct download.
