<div align="center">

<img width="128px" src="assets/images/icon.png" />
<h1><b>Vleer</b></h1>

A free, open-source music app powered by the OpenMusic API Spec—the open standard of music streaming. Access your local library, self-hosted servers, or any compatible source, all without subscriptions.

<a href="https://docs.vleer.app">Docs</a> · <a href="https://discord.gg/invites/PEX37vvWyU">Discord</a>

</div>
<br>

###### This project is in active developement. 4th rewrite btw .\_.

![hero](https://github.com/user-attachments/assets/c86a1abf-12bb-4537-8144-8c8e0c0afdea)

## Roadmap

- [ ] Stable local music player https://github.com/vleerapp/vleer/issues/46
- [ ] OpenMusic API integration

## Installation & Security

### Verifying Downloads

**Simple method** - Download your installer and `SHA256SUMS.txt`, then check the hash:

- Linux/macOS: `sha256sum -c SHA256SUMS.txt --ignore-missing`
- Windows: `certutil -hashfile vleer_setup.msi SHA256`

**GPG verification** - For advanced users who want cryptographic proof:

```bash
# Import our signing key (Key ID: 852407171261C8A4)
gpg --locate-keys hello@vleer.app

# Verify the fingerprint matches: 7E48 1786 6409 4A19 EF60  EEC8 8524 0717 1261 C8A4
gpg --fingerprint 852407171261C8A4

# Verify your download
gpg --verify vleer_setup.msi.asc vleer_setup.msi
```

### First-Time Installation Warnings

<details>
  <summary><kbd>macOS</kbd></summary>
  When you try to open the app, you'll get a warning that the app is damaged. Open the terminal and run this command <code>xattr -dr com.apple.quarantine /Applications/Vleer.app</code> after that it should open without any problems. 
</details>

<details>
  <summary><kbd>Windows</kbd></summary>
  Click "More info" then "Run anyway" to bypass SmartScreen.
</details>

These warnings appear because we don't pay Apple/Microsoft for certificates. The GPG signatures above prove the files haven't been tampered with.
