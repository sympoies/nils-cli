# macos-agent

## 概覽
`macos-agent` 是 macOS 專用的 CLI，自動化桌面 UI 操作（切換視窗、點擊、輸入、快捷鍵、截圖觀察）。
本文件定義 Task 1.1 的穩定合約：命令面、機器可讀輸出、stdout/stderr 行為與 exit code。

## Usage
```text
Usage:
  macos-agent [--format <text|json|tsv>] [--dry-run] <group> <command> [options]

Groups:
  preflight   check runtime dependencies and permissions
  windows     list windows
  apps        list running apps
  window      activate a target window/app
  input       click | type | hotkey
  observe     screenshot

Help:
  macos-agent --help
  macos-agent <group> --help
  macos-agent <group> <command> --help
```

## Command Surface

### `preflight`
- `macos-agent preflight [--strict]`
- 用途：檢查 `osascript`、`cliclick`、Accessibility/Automation（及必要時 Screen Recording）狀態。

### `windows`
- `macos-agent windows list [--app <name>] [--window-name <name>] [--on-screen-only] [--format <json|tsv>]`
- 用途：列出可選視窗目標。

### `apps`
- `macos-agent apps list [--format <json|tsv>]`
- 用途：列出可操作 app 目標。

### `window`
- `macos-agent window activate (--window-id <id> | --active-window | --app <name> [--window-name <name>] | --bundle-id <bundle_id>) [--wait-ms <ms>]`
- 用途：切換前景 app/視窗，作為後續 `input` 命令上下文。

### `input`
- `macos-agent input click --x <px> --y <px> [--button <left|right|middle>] [--count <n>]`
- `macos-agent input type --text <text> [--delay-ms <ms>] [--enter]`
- `macos-agent input hotkey --mods <cmd,ctrl,alt,shift,fn> --key <key>`

### `observe`
- `macos-agent observe screenshot (--window-id <id> | --active-window | --app <name> [--window-name <name>]) [--path <file>] [--image-format <png|jpg|webp>]`
- 用途：抓取目前目標視窗畫面作為可驗證 artifact。

## Selector / Format Rules
- `window activate` 與 `observe screenshot` 的 selector 必須且只能提供一組。
- `--window-name` 只能搭配 `--app`。
- `--format tsv` 只允許 `windows list`、`apps list`；其他命令使用 `--format tsv` 視為 usage error（exit 2）。

## Output Contract

### stdout（穩定）
- 成功時：stdout 僅輸出「命令結果 payload」並以 `\n` 結尾。
- 失敗時：stdout 必須為空。
- stdout 不得混入 log/progress/debug 訊息。

### stderr（穩定）
- 錯誤與提示只走 stderr。
- 禁止 stack trace、panic dump、非結構化除錯噪音。
- `--format json` 時，錯誤 payload 以單一 JSON 物件輸出到 stderr。

### Machine-readable mode（穩定）
- `--format json`：所有命令都可用，且為推薦的機器整合模式。
- `--format tsv`：只供 `windows list` 與 `apps list`。
- `--format text`：人類可讀，不保證跨版本文字完全不變；腳本請改用 `json/tsv`。

### JSON envelope（`--format json`）
成功輸出（stdout）：
```json
{
  "schema_version": 1,
  "ok": true,
  "command": "<group.command>",
  "result": {}
}
```

失敗輸出（stderr）：
```json
{
  "schema_version": 1,
  "ok": false,
  "command": "<group.command>",
  "error": {
    "code": "<stable_snake_case>",
    "message": "<human_readable_message>",
    "hint": "<optional_hint>"
  }
}
```

相容性規則：
- `schema_version` 目前固定為 `1`。
- 破壞性變更必須升版 `schema_version`。
- 非破壞性擴充可新增欄位；consumer 應忽略未知欄位。

### List TSV contract（`windows/apps list --format tsv`）
- 編碼：UTF-8，無 header，一列一筆。
- 欄位中的 tab/newline 會正規化為單一空白。
- 排序需 deterministic。

`windows list` 欄位順序：
1. `window_id`
2. `owner_name`
3. `window_title`
4. `x`
5. `y`
6. `width`
7. `height`
8. `on_screen`

`apps list` 欄位順序：
1. `app_name`
2. `pid`
3. `bundle_id`

## Exit Codes
- `0`: success（命令完成且輸出符合契約）。
- `1`: runtime failure（依賴缺失、權限不足、後端執行失敗、timeout 等）。
- `2`: usage error（參數錯誤、selector 衝突、格式不支援、目標歧義等）。

## Parseable Examples

Window switching (`window activate`):
```bash
macos-agent window activate --app Terminal --wait-ms 1500 --format json
{"schema_version":1,"ok":true,"command":"window.activate","result":{"app":"Terminal","window_id":4811,"wait_ms":1500}}
```

Click (`input click`):
```bash
macos-agent input click --x 200 --y 160 --format json
{"schema_version":1,"ok":true,"command":"input.click","result":{"x":200,"y":160,"button":"left","count":1}}
```

Type (`input type`):
```bash
macos-agent input type --text "hello world" --format json
{"schema_version":1,"ok":true,"command":"input.type","result":{"text_length":11,"submitted_enter":false}}
```

Hotkey (`input hotkey`):
```bash
macos-agent input hotkey --mods cmd,shift --key 4 --format json
{"schema_version":1,"ok":true,"command":"input.hotkey","result":{"mods":["cmd","shift"],"key":"4"}}
```
