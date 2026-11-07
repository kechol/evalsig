# frozen_string_literal: true

# Homebrew formula for evalsig, built from the release's source tarball. It is
# published as Formula/evalsig.rb in github.com/kechol/homebrew-tap.
# At each release, set `url` to the tag and `sha256` to the tarball's checksum
# (the zeros below are a placeholder, not a checksum).
class Evalsig < Formula
  desc "Statistics for the evaluation records of non-deterministic judges"
  homepage "https://github.com/kechol/evalsig"
  url "https://github.com/kechol/evalsig/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/kechol/evalsig.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/evalsig-cli")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/evalsig --version")
  end
end
