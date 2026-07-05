class WrecCli < Formula
  desc "The most efficient screen recorder for mac, CLI runtime"
  homepage "https://wrec-beta.vercel.app"
  version "0.1.1"
  url "https://github.com/shivamhwp/wrec/releases/download/v#{version}/wrec-cli-aarch64-apple-darwin.tar.gz"
  sha256 "bd5f297ed722797c4453e1351cc6af8f434ecd9a1eaa82ea3078d1975e854174" # replaced by scripts/update-homebrew.sh
  license "MIT"

  depends_on :macos
  depends_on arch: :arm64

  def install
    libexec.install "wrec", "daemon", "capture-engine"
    bin.write_exec_script libexec/"wrec"
  end

  test do
    system bin/"wrec", "-V"
  end
end
