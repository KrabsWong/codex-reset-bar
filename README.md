# Codex Resets Bar

macOS 菜单栏应用，用于查看 [Codex Resets](https://codex-resets.com) 的重置记录。
界面由 [Native Shell](https://nsh.dev) 的 Rust 组件绘制，AppKit 提供菜单栏图标和弹出面板。

## 功能

- 最近一次重置时间、累计次数、平均间隔和最长等待时间。
- 近 20 周的活动日历：蓝色为 regular，橙色为 banked，灰色为无重置事件。
- 可滚动的原文记录，最多显示三行，悬停查看全文，点击打开公告来源。
- 仅展示接口明确提供的重置计划，不自行预测下次重置日期。
- 英文界面、系统深浅色、手动刷新、本地缓存和来源署名。
- 标题、头像和底部来源链接打开网站；介绍中的 @thsottiaux 打开其 X 页面。

## 构建与运行

需要 macOS 14+、Xcode 命令行工具和 Rust 工具链。

```sh
cargo run
bash scripts/package.sh --run
```

打包脚本生成 `dist/Codex Resets.app`，使用本地 ad-hoc 签名，不包含 Developer ID 签名或公证。
构建架构取决于 Rust 目标；当前提供的分发包为 Apple Silicon（arm64）。
应用不显示 Dock 图标。点击菜单栏头像展开或收起；按 Esc 或点击面板外可关闭面板。
如需结束应用，可使用 macOS 活动监视器。

生成可分发 ZIP：

```sh
ditto -c -k --sequesterRsrc --keepParent 'dist/Codex Resets.app' dist/Codex-Resets-0.1.0-macOS-arm64.zip
```

## 数据

数据来自 Codex Resets 的公开接口：

- `GET /api/v1/status`：最新重置、已宣布计划、观察状态及汇总统计。
- `GET /api/v1/resets?limit=30&order=desc`：按游标分页读取记录。

刷新时加载完整历史后才计算 Longest wait：按时间排序，取相邻事件之间的最大间隔。
历史不完整或日期无效时不展示部分数据算出的最大值。日历按本地日期分组。

缓存位于 `~/Library/Application Support/CodexResets/cache.json`，启动时先展示缓存，再按需刷新。
网络失败保留已读取的数据。打开期间定期检查状态，面板关闭时不启动周期性刷新。
请求使用 `Cache-Control: no-cache` 重新校验服务端数据。

网站头像用于应用图标、菜单栏和面板标题，原始来源见 [resources/README.md](resources/README.md)。
本项目是独立客户端，数据服务和头像归其各自提供方。

## 开发验证

```sh
cargo test
cargo run -- --snapshot /tmp/panel.png 5
cargo run -- --snapshot /tmp/dark.png 5 --dark
cargo run -- --self-test --redraw-test
CODEX_RESET_SHOW_ON_LAUNCH=1 cargo run
```

可选环境变量：

- `CODEX_RESET_API_BASE`：测试服务地址。
- `CODEX_RESET_CACHE_PATH`：隔离测试缓存路径。
- `CODEX_RESET_TRACE=1`：将刷新日志写入 `/tmp/codex-reset-trace.log`。

## 代码结构

- `src/main.rs`：AppKit 宿主、弹窗定位、重绘和运行时自检。
- `src/ui.rs`：NSH 组件、样式、刷新与分页。
- `src/api.rs` / `src/model.rs`：接口、数据类型和最长等待统计。
- `src/calendar.rs` / `src/format.rs`：活动日历、日期与数字格式。
- `src/cache.rs`：缓存读写。
- `scripts/package.sh` / `scripts/make-icon.swift`：构建应用包与图标。

源码许可见 [LICENSE](LICENSE)。
