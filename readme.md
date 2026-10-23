# course2md-learning

把课程视频整理为截图、课堂文字和可自定义的 AI 学习讲义。

这是基于 [mizorewww/course2md](https://github.com/mizorewww/course2md)
`v2.0.0-rc.11` 的二次开发版本，**不是原作者的官方发行版**。原项目
作者与贡献者的版权、Git 历史和 [MIT 许可证](LICENSE) 均予保留。
具体来源、修改范围见 [NOTICE](NOTICE.md)，第三方资源见
[THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES.md)。

## 本版本的主要调整

- 转录、文字校对、开头摘要和 AI 学习讲义分别配置。
- 添加课程时选择本次 AI 服务和模型；教学提示词可以修改，也可保存默认。
- 转录完成后，按本次设置自动生成结合截图的分步讲义，保留课堂笔记。
- 讲义显示实际完成进度，已完成结果可直接打开，重新生成是独立操作。
- 本地 KaTeX 排版公式，支持分式、积分、矩阵、上下标等。
- 调整原生桌面的布局、服务选择、导航、滚动区域和存储设置。
- 支持 OpenAI 兼容服务、Ollama，以及程序现有的 Codex 订阅连接方式。
  实际可用模型、认证方式和额度由所选服务决定，不代表本项目提供免费 AI。

## 本仓库包含什么

| 目录 | 内容 |
| --- | --- |
| `src` / `tests` | Rust 转换引擎及测试 |
| `desktop` | 原生 GPUI 桌面程序、资源、依赖补丁与构建脚本 |
| `03_工具/学习讲义生成器` | Python 讲义工具、通用教学模板、数学排版和离线测试 |
| `03_工具/项目管理` | 讲义工具需要的存储配置辅助代码 |
| `tools/windows/PortableLauncher.cs` | Windows 便携目录初始化与启动器源码 |

不包含课程、PPT、笔记、私人提示词、API Key、账号凭据、模型权重、缓存
或预编译程序。`prompt43.txt` 是兼容旧接口的文件名，仓库内内容是通用
模板；每位使用者可以自行设置教学提示词。

本次仅提交源码，没有发布二次开发版安装包。原项目发行说明单独保留在
[README.upstream.md](README.upstream.md)，其中下载链接属于上游版本。

## 构建

需要 Rust stable、Git、Python 3.11+。Windows 需要 C++ Build Tools 和
LLVM；其他平台的系统依赖及详细步骤见 [desktop/README.md](desktop/README.md)
与 `.github/actions/desktop-deps/action.yml`。讲义公式排版需要 PATH 中的
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

## 使用与再发布

请保留 LICENSE、NOTICE 和第三方资源的原始授权文件，清楚注明上游
来源与本版本修改。课程资源不因程序开源而获得转载授权。

FFmpeg、模型权重和其他运行时各有许可；公开分发包含这些组件的二进制
包，需要按所用版本另行满足相应条件。本源码仓库的 MIT 许可不会覆盖
它们的许可，也不代表获得原作者、模型厂商或其他品牌的背书。
