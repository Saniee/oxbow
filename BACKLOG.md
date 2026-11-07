# Backlog

Ideas and follow-ups not being worked on yet.

## Updates

- **Updating a .deb** from Oxbow: it only links to the release now, as installing needs root (`pkexec apt
  install`, or an apt repository so the system's updates include Oxbow).

## Releases

- **Code signing** for Windows, so SmartScreen stops warning about the installer: SignPath (free for open source) or
  Microsoft Trusted Signing.
- **winget** package, from the release's installer.
- **ARM64** builds for Windows and Linux (GitHub has ARM runners).
- **More Linux packages** if asked for: `.rpm`, an AUR package. Flatpak only if its sandbox can be made to work
  with the system's git, SSH agent and credential helpers.
