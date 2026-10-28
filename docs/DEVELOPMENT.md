# 开发、运行与测试

[返回项目首页](../readme.md) · [先下载源码](INSTALL.md) · [仓库结构与资料边界](REPOSITORY_GUIDE.md)

## 构建

需要 Rust stable、Git、Python 3.11+。Windows 需要 C++ Build Tools 和
LLVM；其他平台的系统依赖及详细步骤见 [desktop/README.md](../desktop/README.md)
与 [desktop-deps/action.yml](../.github/actions/desktop-deps/action.yml)。讲义公式排版需要 PATH 中的
Node.js；视频处理另外安装 FFmpeg，在线链接另外安装 yt-dlp。

```powershell
python desktop/scripts/sources.py --locked
cargo build
cargo build --manifest-path desktop/Cargo.toml
```

GPUI 依赖由准备脚本按固定提交获取到 `desktop/.deps`，不把整份依赖
检出目录、工具链或第三方可执行文件上传到本仓库。

### Windows 开发运行示例

在仓库根目录运行以下 PowerShell。示例使用 `.local` 隔离运行数据，
不会从其他用户的课程库或账号配置导入内容。

```powershell
$repo = (Get-Location).Path
$configDir = Join-Path $repo '.local/config/course2md'
$courseDir = Join-Path $repo '.local/courses'
$studyDir = Join-Path $repo '.local/study'
New-Item -ItemType Directory -Force $configDir,$courseDir,$studyDir | Out-Null
$configFile = Join-Path $configDir 'config.toml'
if (!(Test-Path -LiteralPath $configFile)) {
    '[defaults]' + "`n" + 'out = ' + (ConvertTo-Json $courseDir -Compress) |
        Set-Content -LiteralPath $configFile -Encoding utf8
}
@{
    config_path = $configFile
    course_root = $courseDir
    outputs_root = $studyDir
    model = ''
} | ConvertTo-Json | Set-Content -LiteralPath '.\03_工具\学习讲义生成器\settings.json' -Encoding utf8
$env:XDG_CONFIG_HOME = Join-Path $repo '.local/config'
$env:XDG_CACHE_HOME = Join-Path $repo '.local/cache'
$env:COURSE2MD_BIN = Join-Path $repo 'target/debug/course2md.exe'
$env:COURSE2MD_STUDY_SCRIPT = Join-Path $repo '03_工具/学习讲义生成器/study.py'
$env:COURSE2MD_STUDY_PYTHON = (Get-Command python).Source
& '.\desktop\target\debug\course2md-desktop.exe'
```

自定义了 `CARGO_TARGET_DIR` 时，需要按实际构建目录修改两处可执行文件
路径。AI 服务在软件中配置，不能把真实密钥写入示例或 Git 提交。

`PortableLauncher.cs` 服务于便携包的相对目录布局，不能直接替代上面的
开发启动步骤。发布含第三方运行时的便携包前，需另行完成依赖分发审查。

## 检查与测试

```powershell
cargo test --lib
cargo test --test cli_ux --test login_cli --test task_execution
cargo test --manifest-path desktop/Cargo.toml
python -X utf8 -B -m unittest discover -s '.\03_工具\学习讲义生成器' -p 'test_*.py'
```

讲义工具测试使用合成图文和模拟模型，不发送付费 AI 请求。跨平台的
完整检查以仓库 CI 和上游构建说明为准；未逐一验证所有系统的安装流程。

