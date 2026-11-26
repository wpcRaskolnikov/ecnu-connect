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

### 搭配 Clash 使用

`ecnu-connect` 可以和同一台电脑上的 Clash / Mihomo 配合使用。下面是一份以 Mihomo 为例的分流配置：VPN 网关和登录流量直连，指定的校园网段走本机 SOCKS5，其余流量直连。

```yaml
mode: rule
find-process-mode: strict

proxies:
  - name: ECNU
    type: socks5
    server: 127.0.0.1
    port: 1080
    udp: false

rules:
  - PROCESS-NAME,zju-connect,DIRECT
  - PROCESS-NAME,zju-connect.exe,DIRECT
  - DOMAIN,vpn.ecnu.edu.cn,DIRECT
  - DOMAIN,sso.ecnu.edu.cn,DIRECT

  # 只把需要通过 VPN 访问的资源交给 ECNU 节点。
  - IP-CIDR,219.123.1.0/24,ECNU,no-resolve
  - MATCH,DIRECT
```

如果使用 [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev)，也可以在「订阅 → 全局扩展脚本」里加入上面的节点和分流规则，不用逐个修改订阅的 YAML。

在规则模式下，命中校园网规则的流量会自动走本机 `ECNU` 节点，日常流量仍按订阅原有的规则分流，不用来回切换节点。下面的脚本会把新增规则放在订阅规则之前，并保留原有的 `MATCH`。

```js
function main(config) {
  const name = "ECNU";
  const proxy = {
    name,
    type: "socks5",
    server: "127.0.0.1",
    port: 1080,
    udp: false,
  };
  const rules = [
    "PROCESS-NAME,zju-connect,DIRECT",
    "PROCESS-NAME,zju-connect.exe,DIRECT",
    "DOMAIN,vpn.ecnu.edu.cn,DIRECT",
    "DOMAIN,sso.ecnu.edu.cn,DIRECT",
    `IP-CIDR,219.123.1.0/24,${name},no-resolve`,
  ];

  config.proxies = [...(config.proxies ?? []).filter(node => node.name !== name), proxy];
  config.rules = [...rules, ...(config.rules ?? []).filter(rule => !rules.includes(rule))];
  return config;
}
```

脚本会替换同名的 `ECNU` 节点；如果订阅里已有同名节点且想保留它，请改用其他名称。保存后重新应用订阅并切换到规则模式。
