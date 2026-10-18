# Reading the commit list

Each row in the **Commits** panel is one commit: its short ID (SHA), the author's initials, the message, how long ago it was made, and any branches or tags pointing at it.

**The icon colour tells you where the commit is:**

- 🔴 **Red**: only on your computer. Nobody else has it yet. You can change it freely.
- 🟡 **Yellow**: pushed to your branch on the remote. Others may have it; changing it means force pushing.
- 🟢 **Green**: already on the main branch (`main` or `master`). Avoid changing it.

SlothGit asks before changing yellow or green commits.

**In the Branches panel**, a branch shows how it compares with its remote copy:

- **↑2**: you have 2 commits the remote doesn't. **Push** sends them.
- **↓3**: the remote has 3 commits you don't. **Pull** (or **Fast-Forward**) brings them in.
- **✓**: both are the same.
- **upstream gone**: the remote branch was deleted, often because it was merged.

Hover over any commit for the full message, date and author.
