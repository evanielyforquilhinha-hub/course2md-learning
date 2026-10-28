# 视频转笔记：下载与配置

[返回首页](../readme.md) · [构建与启动](DEVELOPMENT.md)

想先试试这个工具？先选一种获取源码的方式，再准备运行环境。
**这个仓库目前没有本修改版的安装包；ZIP 里是代码，不能直接双击运行。**

## 方式一：让 AI 编程助手帮忙

需要一个能在你电脑上读写文件、运行命令的编程助手。把
[首页这段要求](../readme.md#agent-setup) 复制给它即可。它仍然需要下载
构建工具和依赖，耗时取决于网络与电脑配置；遇到系统安装界面时可能需要你操作。

如果你只有普通的网页聊天，也可以请 AI 逐步解释本教程，由你自己执行命令。

## 方式二：手动下载，不输入命令

1. 打开 [项目首页](https://github.com/evanielyforquilhinha-hub/course2md-learning)。
2. 点击文件列表上方绿色的 **Code**，再点 **Download ZIP**。也可以
   [直接下载源码 ZIP](https://github.com/evanielyforquilhinha-hub/course2md-learning/archive/refs/heads/main.zip)。
3. 下载完成后，右键 ZIP，选择“全部解压缩”，放到一个新的文件夹中。
4. 打开解压后的 `course2md-learning-main`。能看到 `readme.md`、`Cargo.toml`
   和 `desktop` 文件夹，就说明代码已解压到位。
5. 接着按下文准备环境，再看 [构建与启动](DEVELOPMENT.md)。

“Download ZIP”获取的是当前代码快照；它不包含 Git 历史、模型权重和
个人配置。以后想经常更新代码，推荐改用 Git 克隆。

<a id="commands"></a>

## 方式三：用 PowerShell 下载

Windows 用户可以用这个方法，不需要先安装 Git 来下载 ZIP。
**后面的源码构建仍然需要 Git。**

先打开一个用于保存项目的**新空文件夹**，在资源管理器地址栏输入
`powershell`，按回车。确认窗口中的路径正确后，复制运行：

```powershell
if ((Test-Path -LiteralPath '.\course2md-learning-source.zip') -or (Test-Path -LiteralPath '.\source')) {
    throw '这个目录已有下载内容，请换一个空文件夹。'
}
Invoke-WebRequest -Uri 'https://github.com/evanielyforquilhinha-hub/course2md-learning/archive/refs/heads/main.zip' -OutFile '.\course2md-learning-source.zip' -ErrorAction Stop
Expand-Archive -LiteralPath '.\course2md-learning-source.zip' -DestinationPath '.\source' -ErrorAction Stop
Set-Location -LiteralPath '.\source\course2md-learning-main'
Get-Item -LiteralPath '.\readme.md', '.\Cargo.toml', '.\desktop'
```

窗口列出这三个项目，说明下载和解压成功。代码位于当前目录下的
`source\course2md-learning-main`；下一步是准备环境和构建，尚未安装好软件。

## 方式四：用 Git 克隆，方便以后更新

需要先安装 [Git](https://git-scm.com/downloads)。在打算存放项目的文件夹
打开终端，运行：

```powershell
git clone https://github.com/evanielyforquilhinha-hub/course2md-learning.git
Set-Location -LiteralPath '.\course2md-learning'
git status
```

看到 `On branch main` 和干净的工作目录，说明源码已获取。
如果已有同名文件夹，请选择其他位置，避免和已有项目混在一起。

以后更新：在这个克隆目录打开终端，先用 `git status` 检查是否有自己的
改动；确认没有后运行 `git pull --ff-only`，再重新构建。自己改过的文件
先提交或备份；更新遇到冲突时不要强制覆盖。

## 代码下载好了，下一步做什么？

“构建”就是把代码编译成电脑能运行的程序。这里使用 Rust 原生桌面技术，
第一次需要准备编译环境，不是运行 `npm install` 的网页项目。

| 准备什么 | 为什么需要 |
| --- | --- |
| Rust stable、Git、Python 3.11+ | 编译引擎和桌面程序，准备固定版本的界面依赖 |
| Windows：Visual Studio C++ Build Tools、LLVM | 构建原生 Windows 程序 |
| macOS：Xcode Command Line Tools | 构建原生 macOS 程序 |
| Linux：仓库列出的系统开发包 | 见 [系统依赖配置](../.github/actions/desktop-deps/action.yml) |
| FFmpeg | 读取视频、提取音频和画面 |
| yt-dlp（处理在线链接时） | 获取支持的视频平台内容 |
| Node.js（生成带公式的学习讲义时） | 运行讲义中的公式排版工具 |

依赖会占用额外磁盘空间；选择本地语音识别时，还可能下载体积较大的模型。
在线 AI 需自行配置服务和账号，费用以服务商为准。

1. 按 [开发文档](DEVELOPMENT.md#构建) 准备环境，并在仓库根目录执行构建命令。
2. Windows 按 [隔离配置启动示例](DEVELOPMENT.md#windows-开发运行示例) 启动。
   示例将测试笔记和配置放在仓库内的 `.local`，不会读取其他人的课程库。
3. 看到软件窗口后，回到 [首次使用教程](../readme.md#first-use)。先用一段
   自己有权使用的短视频生成普通笔记，确认画面和文字正常，再配置 AI。

其他系统的原生启动说明见 [桌面开发文档](../desktop/README.md)。完整依赖
安装与构建尚未在所有系统上逐一实测，遇到错误请保留命令和报错内容。

## 常见问题

**为什么 ZIP 里没有 exe？** 这是源码下载。当前没有发布本修改版的
Windows 安装包，不能把源码 ZIP 当作成品软件。原项目的安装包属于
上游版本，不包含这里全部修改。

**下载失败怎么办？** 先确认浏览器能打开项目页面，再重试。网络下载
中断后，建议换一个新空文件夹重新下载，避免把未下载完整的 ZIP 继续解压。

**提示找不到 git、python 或 cargo？** 对应工具尚未安装，或终端还没有
读到新环境变量。安装后重新打开终端，再按开发文档检查。

**可以直接把自己的整个软件目录发给别人吗？** 分享这个公开仓库链接
即可。个人运行目录可能带有密钥、登录状态、笔记和课程；这些不属于
源码，也不应该随软件分享。
