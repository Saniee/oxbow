# Backlog

Ideas and follow-ups not being worked on yet.

## Updates

- **Check for updates:** a button (and maybe a quiet check on start) that asks GitHub Releases for a newer version.
  When there is one, download that platform's installer and run it: the Windows installer closes Oxbow, updates
  it in place (same AppId) and starts it again. On Linux, point to the new AppImage or `.deb`.

## Releases

- **Code signing** for Windows, so SmartScreen stops warning about the installer: SignPath (free for open source) or
  Microsoft Trusted Signing.
- **winget** package, from the release's installer.
- **ARM64** builds for Windows and Linux (GitHub has ARM runners).
- **More Linux packages** if asked for: `.rpm`, an AUR package. Flatpak only if its sandbox can be made to work
  with the system's git, SSH agent and credential helpers.
