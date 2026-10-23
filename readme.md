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

## 开始使用

这是源码仓库，使用前需要构建；当前没有本二次开发版的安装包。

```powershell
git clone https://github.com/evanielyforquilhinha-hub/course2md-learning.git
cd course2md-learning
python desktop/scripts/sources.py --locked
cargo build
cargo build --manifest-path desktop/Cargo.toml
```

先安装对应系统的构建依赖，再执行以上命令。完整环境要求、Windows
首次启动和测试步骤见 [开发、运行与测试](docs/DEVELOPMENT.md)。

- [仓库结构与资料边界](docs/REPOSITORY_GUIDE.md)
- [原生桌面开发](desktop/README.md)
- [讲义工具说明](03_工具/学习讲义生成器/README.md)

## 使用与再发布

请保留 LICENSE、NOTICE 和第三方资源的原始授权文件，清楚注明上游
来源与本版本修改。课程资源不因程序开源而获得转载授权。

FFmpeg、模型权重和其他运行时各有许可；公开分发包含这些组件的二进制
包，需要按所用版本另行满足相应条件。本源码仓库的 MIT 许可不会覆盖
它们的许可，也不代表获得原作者、模型厂商或其他品牌的背书。
