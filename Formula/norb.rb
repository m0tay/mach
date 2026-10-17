class Norb < Formula
  desc "Lightweight, zero-overhead macOS virtualization and container engine in Rust"
  homepage "https://github.com/douglaslobo/norb"
  url "file:///Users/douglaslobo/Developer/norb", using: :git, branch: "main"
  version "0.1.0"
  license "MIT"
  head "https://github.com/douglaslobo/norb.git", branch: "main"

  depends_on :macos
  depends_on arch: :arm64

  env :userpaths

  def install
    ENV.prepend_path "PATH", "#{Dir.home}/.cargo/bin"
    ENV.prepend_path "PATH", "/opt/homebrew/bin"
    system "cargo", "build", "--release", "--bin", "norb"
    bin.install "target/release/norb"
    bin.install_symlink "norb" => "norbctl"

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

      To initialize your environment (Debian Linux, Kubernetes, and Docker):
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
