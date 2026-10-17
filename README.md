# LocalSend-Win32

用 **windows-rs (Rust)** + **原生 Win32 API** 重新实现的 LocalSend 客户端，
完整覆盖官方 v2.2 协议，UI 全部采用 Win32 原生窗口与控件（不模仿官方 Flutter 样式与动画）。

> 当前环境无 Rust 工具链，本工程为「可直接在 Windows 上 `cargo build`」的完整源码。
> 大部分 API 已按 windows-rs 0.52 / axum 0.7 / reqwest 0.12 的惯用法编写；若你本机
> crate 版本解析不一致，按编译器提示微调版本即可（见末尾「版本说明」）。

## 构建与运行

```powershell
# 需要 Windows + Rust (stable) + Visual Studio 构建工具 (MSVC)
cargo build --release
# 生成的 exe 在 target/release/localsend-win32.exe
```

- 程序以 `#![windows_subsystem = "windows"]` 编译，无控制台窗口。
- 日志写入配置文件同目录的 `localsend.log`。
- 配置文件：`%AppData%\LocalSendWin32\config.json`（首次运行自动生成）。

## 功能映射

| 官方功能 | 本实现 |
| --- | --- |
| 文件发送界面 | `ui/send_window.rs`：设备列表 + 待发送文件列表 + 发送/选择/刷新/设置 |
| 设置面板 | `ui/settings_window.rs`：别名、端口、组播端口、接收目录、PIN、HTTPS 开关 |
| 接收确认弹窗 | `ui/receive_window.rs`：显示发送方与文件清单，接受/拒绝 |
| 传输进度条 | 原生 `msctls_progress32` 进度条（发送窗 + 接收窗） |
| 系统托盘 + 右键菜单 | `tray.rs`：打开设置 / 快速发送 / 打开传输路径 / 退出 |
| 收到请求通知 | 弹出接收确认窗 + 托盘气球提示 |
| 传输完成通知 | 托盘气球提示（发送完成 / 接收完成） |
| LAN 发现 | `protocol/discovery.rs`：UDP 组播 `224.0.0.167:53317` + `/register` 双向发现 |
| 接收（HTTP 服务端） | `protocol/server.rs`：`prepare-upload` / `upload` / `cancel` / `register` / `info` |
| 发送（HTTP 客户端） | `protocol/client.rs`：准备会话后逐文件流式上传并回报进度 |

## 架构

- **UI 线程**：单线程 Win32 消息循环，拥有所有窗口与托盘图标。
- **网络线程**：独立 Tokio 运行时运行发现、接收服务端、发送任务。
- **跨线程通信**：网络侧把数据装箱后通过 `PostMessageW` 投递到 UI 线程，
  UI 侧 `Box::from_raw` 取回并释放（见 `msg.rs` 中的自定义消息与载荷）。

## 启用 HTTPS（TLS）

默认构建使用明文 HTTP 以获得最大兼容性与零 TLS 依赖。官方客户端默认要求加密，
因此提供 `https` 特性开关（自签证书，由 `rcgen` 生成）：

```powershell
cargo build --release --features https
```

> 该特性依赖 `axum-server` + `rustls` + `rcgen`，版本耦合较强；若编译报错，
> 多为这几个 crate 之间的版本错配，按编译器提示统一其版本即可。

## 已知边界 / 后续可完善

- 托盘图标当前使用系统默认图标（`IDI_APPLICATION`）；可替换为自定义 `.ico`。
- 发现目前实现组播 + `/register`（默认方式）；HTTP 遗留扫描（遍历本网段 IP）未实现。
- 接收服务端在用户「接受」前会等待决策（超时 120s 后拒绝），与官方行为一致。
- 通知采用托盘**气球提示**（NIF_INFO）。如需 WinRT **Toast**，可在 `tray.rs` 中
  改用 `windows::UI::Notifications::ToastNotificationManager`（需注册 AUMID）。
- 修改端口 / 加密方式后需重启程序生效（已在设置面板提示）。

## 目录结构

```
src/
  main.rs              入口：AppInner、运行时、消息循环
  config.rs            配置持久化
  msg.rs               自定义窗口消息与跨线程载荷
  util.rs              宽字符/对话框助手
  protocol/
    types.rs           协议 JSON 结构体
    discovery.rs       UDP 组播发现 + /register
    server.rs          接收端 HTTP 服务
    client.rs          发送端 HTTP 客户端
  ui/
    mod.rs             窗口类注册 / 子控件助手 / 公共控件初始化
    send_window.rs     发送主窗口（兼做主窗口与托盘宿主）
    settings_window.rs 设置面板
    receive_window.rs  接收确认 + 进度窗口
  tray.rs              系统托盘、右键菜单、气球通知
```
