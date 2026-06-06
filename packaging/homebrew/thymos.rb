# OpenThymos Homebrew formula.
#
# This is the SEED formula for the tap repo (<owner>/homebrew-tap). It is pinned
# to v0.5.0 with real sha256 sums so `brew install <owner>/tap/thymos` works the
# moment you drop it into the tap — no new release needed. After that, the
# `homebrew` job in .github/workflows/release.yml regenerates it on every tag.
class Thymos < Formula
  desc "Governed-cognition runtime — Intent → Proposal → Commit"
  homepage "https://github.com/gryszzz/open-thymos"
  version "0.5.0"

  on_macos do
    on_arm do
      url "https://github.com/gryszzz/open-thymos/releases/download/v0.5.0/thymos-v0.5.0-aarch64-apple-darwin.tar.gz"
      sha256 "e6ac0cb0b6e8306da251dae99c06d0bb72817b6650375c9a49e36ac020694a23"
    end
    on_intel do
      url "https://github.com/gryszzz/open-thymos/releases/download/v0.5.0/thymos-v0.5.0-x86_64-apple-darwin.tar.gz"
      sha256 "a56c92564019e1f5fb1598376d03dbf25cf5734ebab0e6ec98550d6182f798fd"
    end
  end

  on_linux do
    url "https://github.com/gryszzz/open-thymos/releases/download/v0.5.0/thymos-v0.5.0-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "dcecba4a00358cc8c135417292ff880b143c156a7bc525dc66b2b7a2e748ae23"
  end

  def install
    bin.install "thymos", "thymos-server"
  end

  test do
    system "#{bin}/thymos", "--help"
  end
end
