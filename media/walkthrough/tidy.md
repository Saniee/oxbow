# Tidy up commits before you share them

While commits are still red (not pushed), you can reshape them so the history tells a clear story. Right-click a commit in the **Commits** panel:

| Action | What it does | When to use it |
| --- | --- | --- |
| **Reword** | Changes the message | You made a typo or the message is unclear |
| **Squash into Previous** | Merges the commit into the one below it; both messages are kept, combined | Two commits are really one change |
| **Fixup into Previous** | Like squash, but this commit's message is thrown away | The commit only fixes a mistake in the one below ("oops, forgot a file") |
| **Drop** | Removes the commit and its changes | The change wasn't needed after all |
| **Move Up / Down** | Changes the order | Related commits should sit together |
| **Add Staged Changes to This Commit** | Puts what you've staged into this older commit | You forgot something in a commit a few steps back |

## Fixup commits: fixing an older commit later

Sometimes you notice a mistake in an older commit while you're in the middle of other work. Instead of fixing that commit right away:

1. Stage the fix and right-click the older commit → **More Rewrite Actions → Create Fixup Commit**. This makes a new commit named `fixup! <original message>` on top.
2. Carry on working. Make as many fixup commits as you like.
3. When you're ready, right-click the original commit → **More Rewrite Actions → Squash In Fixup Commits Above**. Each fixup commit is folded into the commit it fixes, and disappears.

All of these can be taken back with **Undo**.
