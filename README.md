# Delta Bypass for Rust

[English](README.en.md) · 简体中文

基于 Rust 开发的高性能 Platoboost 自动化绕过工具。自动按序通过各阶关卡并提取通行密钥，提供命令行工具（CLI）与 HTTP REST API 服务两种运行模式。

---

## 项目简介

Delta Bypass for Rust 用于自动化处理 Platoboost 验证流程。输入目标链接或通行凭据后，系统自动按序通过各阶关卡，最终返回生成的访问密钥（Key）。

本项目由 **Hasl_Team** 开发，为 Hasl_Team 原版 Python 自动化项目（[Delta-bypass 原项目仓库](https://github.com/AbabaHnb/Delta-bypass)）的 Rust 高性能重构版本，显著提升了端到端处理速度与并发吞吐能力。

## 运行环境与依赖声明

* **无需安装语言运行时**：无需安装 Python、Node.js、.NET 或任何第三方解释器/运行时。
* **零外部动态库依赖**：无需安装 OpenSSL 等密码学库（加密模块采用纯 Rust 原生实现并静态编译进二进制中）。
* **网络与存储要求**：
  * 需要连接外网访问 `auth.platorelay.com`。
  * 程序运行的工作目录需具备**写权限**，系统会自动创建并维护缓存文件 `.key_cache.json`（清理后会自动重建，不影响服务）。

---

## 编译与安装

系统要求：Rust 1.75 或更高版本。

### 通用源码编译

```bash
# 克隆仓库并编译 Release 产物
cargo build --release
```

编译产物路径：`target/release/delta-bypass` (Windows 下为 `delta-bypass.exe`)。编译完成后，仅需提取单一二进制文件即可独立部署。

### GitHub Actions 云端自动编译 (推荐)

项目已配置 GitHub Actions 自动化构建流程（[.github/workflows/build.yml](file:///.github/workflows/build.yml)）。
- **每次 Push 代码 / PR**：云端将自动针对 Windows (x64)、Linux (GNU) 以及 Linux (Musl 纯静态) 编译二进制文件，可在仓库的 **Actions** 标签页下直接下载 Artifacts 产物。
- **发布版本**：推送版本标签（如 `git tag v1.2.0 && git push origin v1.2.0`）时，GitHub 将自动创建 Release 并附带各平台编译好的可执行文件。

### Linux C 语言环境要求

若系统缺少标准 C 编译器，请先安装基础构建工具：

```bash
# Debian / Ubuntu
sudo apt install build-essential

# CentOS / RHEL / Rocky Linux
sudo dnf groupinstall "Development Tools"

# Alpine Linux
apk add build-base
```

### 极佳兼容性：Musl 纯静态编译 (Linux)

为彻底解决旧版 Linux 系统因 `glibc` 版本过低导致的 `GLIBC_2.xx not found` 错误，可生成无任何动态库依赖的纯静态二进制文件：

```bash
rustup target add x86_64-unknown-linux-musl
sudo apt install musl-tools          # Debian/Ubuntu（其他发行版对应安装 musl-gcc）
cargo build --release --target x86_64-unknown-linux-musl
```

产物路径：`target/x86_64-unknown-linux-musl/release/delta-bypass`，可在任意 Linux 发行版（如 CentOS 7、Alpine、Arch）上直接运行。

---

## 性能对比

基于 8 核 CPU（Debian 操作系统）环境下的实测基准数据：

| 测试项目 | Python 原版 | Rust 重构版 | 说明与提升 |
|---|---|---|---|
| 单链端到端全流程耗时 | ~6.8 秒 | **~5.5 秒** | 综合处理速度提升约 20% |

> **说明**：在端到端 ~5.5 秒的耗时中，约 5.0 秒为上游服务端强制要求的关卡冷却时间（不可压缩网络等待），实际计算与本地处理耗时仅约 0.5 秒。

---

## 命令行工具（CLI）指南

### 常用运行示例

> **警告（针对命令行参数）**：目标 URL 包含 `&` 等特殊字符，请在 Windows CMD/PowerShell 及 Linux Shell 中统一使用**双引号**将 URL 包裹，防止参数被 Shell 截断。

```bash
# 绕过指定 Platoboost 链接
delta-bypass "https://auth.platorelay.com/a?d=<通行凭据>"

# 传入通行凭据字符串或包含凭据的文件路径
delta-bypass "<通行凭据>"
delta-bypass tickets.txt

# 批量生成 3 条测试链接并自动执行绕过
delta-bypass --generate 3

# 仅生成测试链接（不执行自动绕过）
delta-bypass --generate 5 --no-auto

# 启动 HTTP API 服务模式
delta-bypass --serve --port 2233
```

### Windows 命令行终端字符编码说明

旧版本 Windows 命令提示符（`cmd.exe`）默认编码非 UTF-8，可能导致日志显示出现乱码或方块。建议通过以下方式处理：
1. **临时切换 UTF-8 编码**：在 CMD 终端中运行 `chcp 65001` 后再启动程序。
2. **推荐终端环境**：使用 **Windows Terminal** 或 **PowerShell 7+**，其默认支持 UTF-8 编码。

### 命令行参数说明表

| 参数项 | 默认值 | 功能说明 |
|---|---|---|
| `<链接/凭据>` | — | 目标 URL、通行凭据字符串或存有凭据的文件路径 |
| `--generate N` / `-g` | 0 | 批量生成 N 条测试验证链接 |
| `--quiet` / `-q` | 关 | 静默模式，仅输出最终 key，隐藏中间过程日志 |
| `--max-rounds N` | 3 | 最大重试轮数（系统会根据服务端返回关卡数自动调整） |
| `--no-auto` | 关 | 搭配 `--generate` 使用，仅生成链接而不执行绕过 |
| `--serve` | 关 | 以 HTTP API 服务模式运行 |
| `--host` | 0.0.0.0 | 服务监听地址（若仅限本地调用建议设为 127.0.0.1） |
| `--port` / `-p` | 2233 | 服务监听端口 |

---

## HTTP API 接口规范

### 启动服务

```bash
delta-bypass --serve --port 2233
```

### 请求示例

```bash
curl -G http://127.0.0.1:2233/delta \
     --data-urlencode "url=https://auth.platorelay.com/a?d=<通行凭据>"
```

### 响应示例

```json
{
  "key": "FREE_xxxxxxxx",
  "cached": false,
  "error": null,
  "made_by": "Hasl_Team",
  "qq_group": "277707901",
  "times": "5.512340000000s"
}
```

### 响应字段 Schema

| 字段 | 类型 | 说明 |
|---|---|---|
| `key` | String \| null | 成功提取的通行密钥，获取失败时返回 `null` |
| `cached` | Boolean | 标识是否命中 24 小时内的历史结果缓存 |
| `error` | String \| null | 错误描述，操作成功时为 `null` |
| `times` | String | 实际绕过执行耗时（若命中缓存，则返回首次成功绕过时的记录耗时） |

### 常见 Error 字典

| 错误信息 | 诊断说明与处理建议 |
|---|---|
| `链接格式无效 / Malformed link` | 请求 URL 格式错误或无法解析出有效通行凭据 |
| `链接无效 / Invalid link: <服务端详情>` | 凭据本身已失效或过期（上游直接拒绝，不进行重试） |
| `绕过失败 / Bypass failed` | 连续执行两次绕过均未能提取 Key（已包含一次自动重试） |
| `内部执行异常 / Internal execution error` | 系统内部发生未预期异常 |
| `未获得结果 / No result returned` | 异步等待通道意外中断或超时 |

---

## 性能优化策略

系统通过以下核心设计最大化压缩处理延迟：

1. **请求发送起点时间计算（缩短 ~0.2 秒）**  
   上游对 5 秒关卡冷却时间的校验基于服务端接收请求的时间戳。系统改为从客户端请求**发出时刻**开始计时，而非等待响应接收完毕，省去了一次网络往返延迟（RTT）。同时引入动态自适应余量调整机制，防范网络抖动导致的“请求过快”判定。

2. **HTTP 长连接复用（缩短 ~0.6 秒）**  
   建立预连接池以保持与上游服务器的通信长连接，相较于每次重新握手大幅降低耗时。

3. **同链接并发合并**  
   同一链接的并发请求只会触发一次真实求解，其余请求共享同一结果，避免重复消耗。

4. **客户端版本自动跟进**  
   提交时携带的 `x-client-version` 必须与上游网页客户端一致，旧了会被直接拒绝（`outdated client`）。系统在启动时与每小时自动抓取上游单页应用脚本，提取版本号并逐个验证，试出可用值即自动更新——上游升级无需改代码重新部署。提取失败时退回内置兜底值。

---

## 生产环境部署与服务化指南

### 1. Windows 服务化部署

#### 方式 A：Windows 任务计划程序（无需额外工具）
1. 打开“任务计划程序”，点击右侧“创建任务”。
2. **常规**页：勾选“不管用户是否登录都要运行”及“使用最高权限运行”。
3. **触发器**页：新建触发器，选择“启动时”。
4. **操作**页：新建操作，“程序或脚本”填写 `delta-bypass.exe` 的绝对路径；“添加参数”填入 `--serve --host 127.0.0.1 --port 2233`；**“起始于”务必填写 exe 所在的完整目录**（若未填写，工作目录会被重定向至 `C:\Windows\System32` 导致缓存写入失败）。
5. **设置**页：勾选“如果任务失败，按以下频率重新启动”。

#### 方式 B：NSSM 服务管理器（推荐：支持自动重启与日志重定向）
从 [nssm.cc](https://nssm.cc) 下载 NSSM 工具，以管理员身份运行 CMD 执行：

```cmd
nssm install DeltaBypass C:\delta-bypass\delta-bypass.exe
nssm set DeltaBypass AppParameters "--serve --host 127.0.0.1 --port 2233"
nssm set DeltaBypass AppDirectory C:\delta-bypass
nssm set DeltaBypass AppStdout C:\delta-bypass\out.log
nssm set DeltaBypass AppStderr C:\delta-bypass\err.log
nssm start DeltaBypass
```

### 2. Linux Systemd 服务部署

可以使用项目自带的服务文件 `deploy/delta-bypass.service`：

```bash
# 1. 创建程序部署目录并复制可执行文件
sudo mkdir -p /opt/delta-bypass
sudo cp target/release/delta-bypass /opt/delta-bypass/
sudo chown -R www:www /opt/delta-bypass

# 2. 配置 Systemd 服务
sudo cp deploy/delta-bypass.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now delta-bypass

# 3. 查看服务状态与日志
systemctl status delta-bypass
journalctl -u delta-bypass -f
```

*若系统不存在服务运行用户 `www`，请先创建无登录权限的系统用户*：
```bash
sudo useradd -r -s /usr/sbin/nologin www
```

### 3. Docker 容器化部署

#### 推荐 Multi-Stage Dockerfile

```dockerfile
# 构建阶段
FROM rust:1-alpine AS builder
RUN apk add --no-cache build-base
WORKDIR /build
COPY . .
RUN cargo build --release

# 运行阶段
FROM alpine:latest
RUN apk add --no-cache ca-certificates && \
    adduser -D -H app
WORKDIR /app
COPY --from=builder /build/target/release/delta-bypass /app/
RUN chown -R app:app /app
USER app
EXPOSE 2233
CMD ["/app/delta-bypass", "--serve", "--host", "0.0.0.0", "--port", "2233"]
```

#### 容器构建与运行命令

```bash
docker build -t delta-bypass .

# 启动容器并挂载数据卷保存 key 缓存
docker run -d --name delta-bypass \
  -p 127.0.0.1:2233:2233 \
  -v delta-keys:/app \
  --restart unless-stopped \
  delta-bypass
```

> **注意**：
> 1. 容器中必须安装 `ca-certificates`，否则无法建立 HTTPS 连接。
> 2. 容器内部 `--host` 需设为 `0.0.0.0`，外部安全限制通过 `-p 127.0.0.1:2233:2233` 绑定至本地回路。

### 4. Nginx 反向代理与安全加固

**安全警告**：本项目 HTTP API 接口默认未配置身份验证机制。任何能够连通该端口的主机均可调用绕过能力。**严禁直接将接口无保护暴露于公网环境。**

建议配置 Nginx 进行访问鉴权与限流：

```nginx
limit_req_zone $binary_remote_addr zone=delta:10m rate=10r/s;

location /delta {
    limit_req zone=delta burst=20 nodelay;
    auth_basic "Delta Bypass for Rust API Authorization";
    auth_basic_user_file /etc/nginx/.htpasswd;
    proxy_pass http://127.0.0.1:2233;
    proxy_read_timeout 120s;
}
```

---

## 项目代码结构

```
src/
├── main.rs              命令行入口（CLI 参数解析与任务调度）
├── lib.rs               核心库入口（模块导出声明）
├── config.rs            系统全局配置项与常量定义
├── api.rs               HTTP API 服务、Key 缓存与并发请求合并机制
├── chain.rs             绕过主流程（提交 -> 关卡递进 -> 密钥提取）
├── auth.rs              与认证服务端通信协议实现
├── crypto.rs            上游自定义加密算法实现
├── net.rs               HTTP 长连接管理
├── useragent.rs         User-Agent 伪装与 Mobile 浏览器特征生成
├── link.rs              测试链接生成器
└── timing.rs            高精度耗时统计与性能诊断跟踪器
```

---

## 核心配置参数

全局参数均定义于 `src/config.rs` 中，关键配置项说明如下：

| 配置常量 | 默认值 | 调整说明与限制 |
|---|---|---|
| `MIN_STEP_GAP` | 5 秒 | 上游关卡强制冷却硬性间隔，**严禁下调** |
| `GAP_MARGIN_START` | 250 毫秒 | 动态自适应延迟余量初始值 |
| `POLL_MAX_ATTEMPTS` | 10 | 密钥轮询最大尝试次数 |
| `MAX_ROUNDS_HARD_CAP` | 12 | 关卡循环硬上限（防止异常链接发生死循环） |

---

## 常见问题与排查指南

| 异常现象 | 根因分析 | 解决方案 |
|---|---|---|
| `链接无效 / Invalid link` | 凭据过期或已被上游拒绝 | 更换新的 Platoboost 测试链接（上游主动拒绝行为） |
| `绕过失败 / Bypass failed` | 流程中某一环节未能成功响应 | 移除 `--quiet` 参数重新运行，检查日志末尾 `终止于:` 确定故障具体步骤 |
| 大量日志提示“触发频率限制” | 关卡冷却间隔过短 | 检查 `MIN_STEP_GAP` 配置；若上游风控升级需提高冷却时间 |
| 高并发下连接频繁失败 | 系统句柄限制（Linux） | 执行 `ulimit -n 65535` 或在 Systemd 服务中配置 `LimitNOFILE=65535` |
| Windows CMD 中文显示乱码 | 终端默认编码非 UTF-8 | 运行 `chcp 65001` 或改用 Windows Terminal / PowerShell 7 |
| Linux 提示 `GLIBC_2.xx not found` | 编译镜像 glibc 版本高过目标系统 | 采用 musl 纯静态编译目标（`x86_64-unknown-linux-musl`） |
| Linux 提示 `Permission denied` | 二进制缺乏可执行权限 | 执行 `chmod +x delta-bypass` 赋予权限 |
| 无法找到 `.key_cache.json` | 启动工作目录非二进制所在路径 | 检查 Task Scheduler 中的“起始于”或 Systemd 中的 `WorkingDirectory` 设置 |
| Docker 容器内部报证书错误 | 镜像缺失 CA 根证书 | 在 Dockerfile 运行时阶段安装 `ca-certificates` |
| 端口号已被占用 | 目标端口被其他进程绑定 | 修改 `--port` 端口号，或使用 `netstat` / `ss` 命令定位并关闭冲突进程 |

---

## 开源协议

本项目基于 [MIT 协议](LICENSE) 开源。