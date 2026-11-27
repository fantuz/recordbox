# Open recordbox on macOS

The GitHub ZIP contains a standalone `Recordbox.app`. It runs without Homebrew, Xcode or Rust. The published app has **no Apple Developer ID signature or notarization ticket**. macOS may ask you to decide whether to trust this particular download. [Official recordbox releases](https://github.com/fantuz/recordbox/releases)

## Verify your download

1. In **Apple menu → About This Mac**, choose the **arm64** ZIP for an Apple silicon chip or the **x86_64** ZIP for an Intel processor.
2. Download the ZIP and `SHA256SUMS` from the **same recordbox release**. Open `SHA256SUMS` in a text editor and find the line with your ZIP's exact filename.
3. In Terminal, type `shasum -a 256` followed by a space. Drag the **ZIP** from Finder into Terminal and press Return. Compare all 64 hexadecimal characters with the value on that line. If they differ, discard the download.
4. Double-click the verified ZIP, move `Recordbox.app` to **Applications**, then double-click the app in Finder. If it opens, setup is complete.

The matching digest confirms you received the archive published with that release. The decision to trust the publisher is yours.

## If macOS cannot verify the developer

If the message says the developer cannot be verified or Apple cannot check the app for malicious software, dismiss the message. If you trust this copy and choose to run it, open **System Settings → Privacy & Security**, scroll to **Security**, find the entry for **Recordbox**, and click **Open Anyway**. Confirm and authenticate if prompted. Apple makes this option available for about an hour after the blocked launch. This approval applies to the selected app. [Apple’s illustrated instructions](https://support.apple.com/en-us/102445#openanyway) · [Mac User Guide](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unknown-developer-mh40616/mac)

If the alert says the app **will damage your computer**, stop. If it says the app **is damaged**, discard that copy and verify a fresh download from the release page. On an organization-managed Mac, your administrator may control which apps you can approve. [Apple’s explanation of the alerts](https://support.apple.com/en-us/102445) · [Apple deployment guidance](https://support.apple.com/guide/deployment/app-security-on-a-mac-dep323ab8aa3/web)

## Homebrew installation

The Homebrew formula builds the tagged source on your Mac and signs the resulting app **ad hoc**. That signature seals the local build’s contents. Apple-issued publisher identification and notarization require separate steps. [Homebrew installation instructions](../README.md#homebrew-on-macos)
