class Mach < Formula
  desc "Supersonic, lightweight Linux virtualization & container engine for Apple Silicon"
  homepage "https://github.com/douglaslobo/mach"
  url "file:///Users/douglaslobo/Developer/norb/dist/mach-0.1.0-aarch64-apple-darwin.tar.gz"
  version "0.1.0"
  sha256 "aa373d84916cbaa519501e0c4cb01a9c65fc01ce22eefd2102b596ef066194cb"
  license "MIT"

  depends_on :macos
  depends_on arch: :arm64

  def install
    bin.install "mach"
    bin.install_symlink "mach" => "machctl"
    bin.install_symlink "mach" => "norb"
    bin.install_symlink "mach" => "norbctl"

    # Install assets
    (pkgshare/"assets").install Dir["assets/*"]

    # Install man pages
    man1.install "man/man1/mach.1"
    man1.install "man/man1/machctl.1"

    # Codesign with hypervisor and virtualization entitlements
    system "codesign", "--force", "--entitlements", "entitlements.plist", "--sign", "-", bin/"mach"

    # Shell autocompletions for bash, zsh, and fish
    (bash_completion/"mach").write Utils.safe_popen_read(bin/"mach", "completion", "bash") rescue nil
    (zsh_completion/"_mach").write Utils.safe_popen_read(bin/"mach", "completion", "zsh") rescue nil
    (fish_completion/"mach.fish").write Utils.safe_popen_read(bin/"mach", "completion", "fish") rescue nil
  end

  def post_install
    # Initialize Mach environment, configs, keys, contexts safely without overwriting user data
    system bin/"mach", "init", "--non-interactive"
  end

  def caveats
    <<~EOS
      ⚡️ Mach is initialized and ready on your Apple Silicon Mac!

      Quickstart:
        • Open Linux shell:         mach
        • Run Linux commands:       mach uname -a
        • Docker containers:        docker ps
        • Kubernetes cluster:       kubectl get nodes
        • Activity dashboard:       machctl top
        • Open files in Finder:     open ~/Mach
        • macOS Menu bar:           mach menu
    EOS
  end

  test do
    system "#{bin}/mach", "--version"
    system "#{bin}/machctl", "doctor"
  end
end
