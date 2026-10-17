class Norb < Formula
  desc "Lightweight, zero-overhead macOS virtualization and container engine in Rust"
  homepage "https://github.com/douglaslobo/norb"
  url "file:///Users/douglaslobo/Developer/norb/dist/norb-0.1.0-aarch64-apple-darwin.tar.gz"
  version "0.1.0"
  sha256 "2c6340352b95adb1f4e0788e43e1ed27a553ad54fe9e6fd6b21b08740363e70d"
  license "MIT"

  depends_on :macos
  depends_on arch: :arm64

  def install
    bin.install "norb"
    bin.install_symlink "norb" => "norbctl"

    # Install assets
    (pkgshare/"assets").install Dir["assets/*"]

    # Codesign with hypervisor and virtualization entitlements
    system "codesign", "--force", "--entitlements", "entitlements.plist", "--sign", "-", bin/"norb"

    # Shell autocompletions for bash, zsh, and fish
    (bash_completion/"norb").write Utils.safe_popen_read(bin/"norb", "completion", "bash")
    (zsh_completion/"_norb").write Utils.safe_popen_read(bin/"norb", "completion", "zsh")
    (fish_completion/"norb.fish").write Utils.safe_popen_read(bin/"norb", "completion", "fish")
  end

  def caveats
    <<~EOS
      🎉 Norb is installed and ready on your Apple Silicon Mac!

      To start and initialize your environment:
        norb

      Quickstart:
        • Drop into Debian shell:   norb
        • Run Linux commands:       norb uname -a
        • Docker containers:        docker ps
        • Kubernetes cluster:       kubectl get nodes
        • Activity dashboard:       norb top
        • Open files in Finder:     open ~/Norb
        • Apple menu bar icon:      norb menu &
    EOS
  end

  test do
    system "#{bin}/norb", "--version"
    system "#{bin}/norbctl", "doctor"
  end
end
