# Reference literature

The agent-simulation source PDFs (e.g. the MATSim book, AI-Metropolis, the
dynamic-LOD AAMAS paper, ScaleSim) are **no longer tracked in git** — large
binaries bloat every clone. They are now `.gitignore`d.

## Getting the PDFs

They remain recoverable from git history (they were removed in the
`chore/meantime-hardening` change, not rewritten out of history):

```bash
# list the last commit that still had them
git log --oneline -- docs/literature/agent-simulation/sources/
# restore one from that commit
git checkout <commit>^ -- docs/literature/agent-simulation/sources/<file>.pdf
```

Or fetch them from their original sources:

- MATSim book — https://www.matsim.org/the-book
- the papers — by title via your preferred academic source

## Note on clone size

Removing the files from `HEAD` stops new clones from checking them out, but the
blobs still live in git history, so `.git` size is unchanged until a deliberate
history rewrite (`git filter-repo`). Do that only when no long-lived branches
are in flight, with team coordination — it rewrites every commit SHA.
