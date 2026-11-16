# version and sha256 are rewritten by .github/workflows/release.yml on every release.
cask "glint" do
  version "0.1.2"
  sha256 "936223c7f4533709760f45602d9cafd31bd6c8c3a19831f8354f49f61c879569"

  url "https://github.com/thevrus/glint/releases/download/v#{version}/Glint_#{version}_universal.dmg"
  name "Glint"
  desc "Menu bar control for Logitech Litra lights"
  homepage "https://github.com/thevrus/glint"

  auto_updates true
  depends_on macos: :sonoma

  app "Glint.app"

  # The app is not notarized yet, so Gatekeeper would refuse to open the quarantined copy.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/Glint.app"]
  end

  zap trash: "~/Library/Application Support/dev.glint.desktop"
end
