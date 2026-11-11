# Pull request media

This branch holds the screenshots, recordings and other media that pull requests show in their descriptions, one
folder per change. It shares no history with `main`, is never merged and runs no workflows.

- Add commits on top only. Never delete this branch, force-push it or rewrite its history: pull requests link to its
  files by commit, and a rewrite would break those links.
- Link a file by the commit that added it:
  `https://raw.githubusercontent.com/chrislaupama/device/<commit>/<folder>/<file>`.
- One folder per change, named after the change (`return-tracks/`); file names say what they show
  (`before-mixer.png`, `after-mixer.png`).
- Only the app's own output and work made for this project; keep each file small.

The full guide (commands, formats, sizes and what goes where) is
[docs/pr-media.md](https://github.com/chrislaupama/device/blob/main/docs/pr-media.md) on `main`.
