# Releasing and Distribution Guide

This document describes how to release a new version of `s7s` and distribute it via a custom Homebrew Tap.

> **A release is not complete until the Homebrew tap is updated.** When asked to
> "deploy" / "release" a version, an agent must run **both** stages below end to
> end: stage 1 publishes the GitHub Release, stage 2 updates the tap so
> `brew upgrade` actually serves the new binaries. Stage 2 lives in a separate
> repository (`ular-io/homebrew-tap`) but is a mandatory part of every release.

## Prerequisites

Ensure you have the following tools installed and authenticated on your local machine:
* **Rust & Cargo**: For compiling source files.
* **GitHub CLI (`gh`)**: For managing tags and releases. Run `gh auth status` to verify credentials.
* **shasum**: For calculating SHA256 checksums (standard on macOS).

---

## 1. Running the Release Script

An automated release script is provided under `scripts/release.sh`. It automatically adds required build targets, compiles release binaries for both ARM64 and AMD64 architectures, builds tarball archives, pushes a git tag, and creates a GitHub Release.

The script creates an **annotated** tag (`git tag -a`) so the Tags/Release page shows the `Release <version>` tag message rather than falling back to the pointed-to commit subject. The GitHub Release body is filled with `--generate-notes` (auto-generated from merged PRs/commits since the previous tag; falls back to a Full Changelog link when there are no PRs).

Bump the version in `Cargo.toml` (and sync `Cargo.lock`), commit, and push to `main` before running the script — the script only pushes the tag, not the branch commit it points to.

To create and publish a new version (e.g., `v0.1.0`):

```bash
./scripts/release.sh v0.1.0
```

Once execution completes, the script will output the SHA256 checksums for both architectures:
```text
--------------------------------------------------
ARM64 SHA256: <ARM64_SHA256_HASH>
AMD64 SHA256: <AMD64_SHA256_HASH>
--------------------------------------------------
```
**Keep these checksums handy**, as you will need them to update the Homebrew Formula.

---

## 2. Updating the Homebrew Formula (mandatory)

The Homebrew tap lives in a **separate repository**: `ular-io/homebrew-tap`
(tap name `ular-io/tap`, formula `Formula/s7s.rb`). Users get the new binaries
only after this formula is updated and pushed. Do **not** hardcode a personal
absolute clone path — resolve the tap location portably.

### Locate the tap checkout

```bash
# If the tap is already installed via Homebrew, this prints its git checkout:
TAP_DIR="$(brew --repository ular-io/tap 2>/dev/null)"
# Otherwise clone it (the assets on the GitHub Release are enough):
[ -d "$TAP_DIR" ] || { gh repo clone ular-io/homebrew-tap /tmp/homebrew-tap && TAP_DIR=/tmp/homebrew-tap; }
cd "$TAP_DIR" && git pull --ff-only
```

### Fields to update in `Formula/s7s.rb`

For the version just released (e.g. `v0.1.2` → formula version `0.1.2`), update
**all five** in tandem:

1. `version "<X.Y.Z>"` — explicit field is required (prevents Homebrew from
   misreading the `64` in `s7s-mac-arm64.tar.gz` as a version).
2. ARM64 `url` — `.../download/v<X.Y.Z>/s7s-mac-arm64.tar.gz`
3. ARM64 `sha256` — the **ARM64** checksum printed by `release.sh`.
4. AMD64 `url` — `.../download/v<X.Y.Z>/s7s-mac-amd64.tar.gz`
5. AMD64 `sha256` — the **AMD64** checksum printed by `release.sh`.

If the checksums are not at hand, recompute them from the published assets:

```bash
V=v0.1.2   # the version being released
for arch in arm64 amd64; do
  echo -n "$arch: "
  curl -sL "https://github.com/ular-io/ular-s7s/releases/download/$V/s7s-mac-$arch.tar.gz" | shasum -a 256 | awk '{print $1}'
done
```

### Formula structure (`Formula/s7s.rb`)

```ruby
class S7s < Formula
  desc "Unified TUI to search and resume Claude Code, Antigravity CLI, and Codex sessions"
  homepage "https://github.com/ular-io/ular-s7s"
  version "0.1.2"
  license "MIT"

  if Hardware::CPU.arm?
    url "https://github.com/ular-io/ular-s7s/releases/download/v0.1.2/s7s-mac-arm64.tar.gz"
    sha256 "<ARM64_SHA256_HASH>"
  else
    url "https://github.com/ular-io/ular-s7s/releases/download/v0.1.2/s7s-mac-amd64.tar.gz"
    sha256 "<AMD64_SHA256_HASH>"
  end

  def install
    bin.install "s7s"
  end

  test do
    system "#{bin}/s7s", "--version"
  end
end
```

### Validate, commit, and push the tap

```bash
ruby -c Formula/s7s.rb                       # syntax check
git add Formula/s7s.rb
git commit -m "chore: release s7s v0.1.2"    # English only (tap repo rule)
git push origin main
```

Now users can install or upgrade the tool by running:
```bash
brew tap ular-io/tap
brew install s7s      # or: brew upgrade s7s
```
