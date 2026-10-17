# LocalSend-Win32

用 **windows-rs (Rust)** + **原生 Win32 API** 重新实现的 LocalSend 客户端，
完整覆盖官方 v2.2 协议，UI 全部采用 Win32 原生窗口与控件。


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
