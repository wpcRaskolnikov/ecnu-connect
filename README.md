# ecnu-connect

华东师范大学（ECNU）校园 VPN 的客户端。它通过打开浏览器完成登录，再通过 [zju-connect](https://github.com/Mythologyli/zju-connect) 提供本机 SOCKS5 和 HTTP 代理。

## 安装

从 [GitHub Releases](https://github.com/wpcRaskolnikov/ecnu-connect/releases) 下载对应平台的发布包：

- **Linux / macOS**：支持 x86_64 和 ARM64，可使用 Shell 安装脚本，也可解压发布包后直接运行。
- **Windows**：支持 x86_64，可使用 MSI 安装包，也可解压发布包后运行 `ecnu-connect.exe`。

发布版已内置 `zju-connect`，无需单独安装。登录需要桌面图形会话，以及 Chrome、Edge 或 Firefox 中的至少一个。

## 使用

```sh
ecnu-connect
```

在打开的学校登录页面输入账号密码，并按页面提示完成验证。登录完成后，把需要访问校园网的应用配置为以下任一代理：

| 类型 | 默认地址 |
| --- | --- |
| SOCKS5 | `127.0.0.1:1080` |
| HTTP | `127.0.0.1:1081` |

客户端不会修改系统代理设置。使用代理时请保持终端中的客户端运行，结束时按 `Ctrl+C`。

```sh
ecnu-connect --browser firefox
ecnu-connect --port 1088 --http-port 1089
ecnu-connect --help
```

`--browser` 接受 `chrome`、`edge` 或 `firefox`；`--port` 指定 SOCKS5 端口，`--http-port` 指定 HTTP 端口，两者不能相同。

如果 Chrome 安装在非标准位置，可以设置 `ECNU_CHROME` 指定其可执行文件路径。
