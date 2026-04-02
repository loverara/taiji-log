# taiji-log

Taiji 日志过滤与 RAW 分组查看工具。

## 安装

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/loverara/taiji-log/main/install.sh | bash
```

指定版本：

```bash
TAIJI_LOG_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/loverara/taiji-log/main/install.sh | bash
```

### Windows（PowerShell）

```powershell
iwr -useb https://raw.githubusercontent.com/loverara/taiji-log/main/install.ps1 | iex
```

指定版本：

```powershell
$env:TAIJI_LOG_VERSION = "v0.1.0"; iwr -useb https://raw.githubusercontent.com/loverara/taiji-log/main/install.ps1 | iex
```

## 使用

```bash
taiji-log logs/taji-2026-03-29.log -r REQUEST_ID -raw-f
cat logs/taji-2026-03-29.log | taiji-log -raw-f
```
