# 📊 git-summary: Git Contribution Report by Author

`git-summary` is a CLI utility for generating author-level contribution summaries in Git.  
It provides commit counts, added/deleted lines, and commit date ranges — grouped by email and sorted by net contribution.

## 💡 Output Preview

```text
Name                      Email                                       Added  Deleted      Net  Commits        First         Last
----------------------------------------------------------------------------------------------------------------------------------------
yourname                  10888888+yourname@users.noreply.github.c     6691     1095     5596       34   2025-06-03   2025-06-09
bob                       bob@gmail.com                                1677      179     1498        5   2024-10-23   2025-06-01
dependabot[bot]           49699333+dependabot[bot]@users.noreply.g                          0        0
```

---

## 📦 Use Cases

- Review who contributed how much in a given time window
- Track team productivity by week, month, or custom period
- Get a lightweight alternative to `git shortlog` with more detail
- Prepare contribution reports or metrics

---

## 🛠 Commands

### `git-summary all`

📅 Show full history of all commits

```bash
git-summary all
```

---

### `git-summary <start> <end>`

📅 Show summary for a custom date range

```bash
git-summary 2024-01-01 2024-12-31
```

Both dates must be in `YYYY-MM-DD` format.

---

### Preset Ranges

#### `git-summary today`

📅 Show contributions from today (local time)

#### `git-summary yesterday`

📅 Show contributions from yesterday

#### `git-summary this-week`

📅 This current week (Monday to Sunday)

#### `git-summary last-week`

📅 Last full week (Monday to Sunday)

#### `git-summary this-month`

📅 From the 1st day of the month to today

#### `git-summary last-month`

📅 Last full month

---

## 🧪 Example

```bash
git-summary this-month
```

---

## 🧱 Implementation Notes

- Timezone correction is applied (UTC+8 default) for date consistency
- Ignores lockfiles by default: `yarn.lock`, `pnpm-lock.yaml`, etc.
- Supports macOS and Linux via `date` command compatibility
- Output is plain-text tabular, ideal for terminal use or redirection
