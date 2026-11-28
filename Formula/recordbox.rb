class Recordbox < Formula
  desc "Local DJ library, track analysis, set preparation and classic USB export"
  homepage "https://github.com/fantuz/recordbox"
  url "https://github.com/fantuz/recordbox.git",
      tag:      "v0.9.3",
      revision: "83ff2e75e9e96b58de4d6bbcd3ca2f8ba51c5577"
  license "MIT"

  depends_on "python@3.13" => :build
  depends_on "rustup" => :build
  depends_on :macos

  def install
    ENV["CARGO_HOME"] = buildpath/"cargo-home"
    ENV["RUSTUP_HOME"] = buildpath/"rustup"
    rustup = Formula["rustup"].opt_bin/"rustup"
    toolchain = (buildpath/"rust-toolchain.toml").read[/^channel = "([^"]+)"$/, 1]
    system rustup, "toolchain", "install", toolchain, "--profile", "minimal"
    system rustup, "run", toolchain, "cargo", "build", "--manifest-path", "app/Cargo.toml",
           "--release", "--locked", "--target-dir", "target"

    revision = stable.specs.fetch(:revision)
    arch = Hardware::CPU.arm? ? "arm64" : "x86_64"
    python = Formula["python@3.13"].opt_bin/"python3.13"
    system python, "packaging/build_archive.py", "--binary", "target/release/recordbox",
           "--platform", "macos", "--arch", arch,
           "--source-commit", revision, "--output", "dist"

    libexec.install "dist/Recordbox.app"
    app = libexec/"Recordbox.app"
    resources = app/"Contents/Resources"
    # The archive manifest hashes the unsigned binary. Keep it outside the signed bundle.
    pkgshare.install resources/"runtime-manifest.json" => "unsigned-build-manifest.json"
    (resources/"DISTRIBUTION.txt").atomic_write("recordbox #{version}, built locally from #{revision} and signed ad hoc.\n")
    %w[CFBundleName CFBundleDisplayName].each do |key|
      system "/usr/bin/plutil", "-replace", key, "-string", "recordbox", app/"Contents/Info.plist"
    end
    system "/usr/bin/codesign", "--force", "--sign", "-", app
    bin.install_symlink app/"Contents/MacOS/recordbox"
  end

  def caveats
    <<~EOS
      To open the locally built app in Finder, use:
        open "#{opt_libexec}/Recordbox.app"
      To start it from the terminal, run recordbox.
    EOS
  end

  test do
    system "/usr/bin/codesign", "--verify", "--strict", libexec/"Recordbox.app"
    assert_match "#{version} · Apple Metal build", shell_output("#{bin}/recordbox --stems-status")
  end
end
