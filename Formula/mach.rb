class Mach < Formula
  desc "Supersonic Linux virtualization & container engine for Apple Silicon"
  homepage "https://github.com/m0tay/mach"
  url "https://github.com/m0tay/mach/releases/download/v0.15.0/mach-0.15.0-aarch64-apple-darwin.tar.gz"
  sha256 "e64d1f429a954fc5114c1dc3530ad5660c53e5d39bfc150130b55ca45e66aa09"
  license "MIT"

  depends_on arch: :arm64
  depends_on :macos

  def install
    bin.install "mach"
    bin.install_symlink "mach" => "machctl"

    # Install assets
    (pkgshare/"assets").install Dir["assets/*"]

    # Install man pages
    man1.install "man/man1/mach.1"
    man1.install "man/man1/machctl.1"

    # Strip quarantine if present
    system "/usr/bin/xattr", "-cr", "."

    # Codesign with hypervisor and virtualization entitlements
    system "codesign", "--force", "--entitlements", "entitlements.plist", "--sign", "-", bin/"mach"

    bash_completion.install "completions/mach.bash"
    zsh_completion.install "completions/_mach", "completions/_machctl"
    fish_completion.install "completions/mach.fish"
  end

  def post_install
    # Initialize Mach environment, configs, keys, contexts safely without overwriting user data
    system bin/"machctl", "init", "--non-interactive"
  end

  def caveats
    <<~EOS
      Mach is installed for your Apple Silicon Mac.

      Quickstart:
        • Open Linux shell:         mach
        • Run Linux commands:       mach uname -a
        • Start Docker:              machctl docker start
        • Enable Kubernetes:         machctl k8s enable
        • Activity dashboard:       machctl top
        • Open files in Finder:     open ~/Mach
        • macOS Menu bar:           mach menu

      To completely uninstall Mach and all stored data:
        machctl uninstall --all
        brew uninstall mach
    EOS
  end

  test do
    system "#{bin}/mach", "--version"
    system "#{bin}/machctl", "doctor"
  end
end
