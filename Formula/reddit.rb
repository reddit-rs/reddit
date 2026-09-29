# The release workflow replaces this source bootstrap with verified binary URLs.
class Reddit < Formula
  desc "Archive Reddit listings as JSON and media with an offline viewer"
  homepage "https://github.com/reddit-rs/reddit"
  url "https://github.com/reddit-rs/reddit.git", tag: "v0.5.0"
  version "0.5.0"
  license any_of: ["MIT", "Apache-2.0"]

  depends_on "cmake" => :build
  depends_on "llvm" => :build
  depends_on "perl" => :build
  depends_on "rust" => :build

  def install
    ENV["LIBCLANG_PATH"] = Formula["llvm"].opt_lib.to_s
    system "cargo", "install", "--locked", "--path", ".", "--root", prefix
  end

  test do
    assert_match "reddit #{version}", shell_output("#{bin}/reddit --version")
  end
end
