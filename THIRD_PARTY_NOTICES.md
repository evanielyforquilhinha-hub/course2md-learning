# 第三方资源与分发范围

主项目的 MIT 许可证不会替换第三方资源的许可证。以下说明与原有的
授权、版权和来源文件共同保留。

| 组件或资源 | 来源与许可说明 |
| --- | --- |
| course2md 原始源码 | [mizorewww/course2md](https://github.com/mizorewww/course2md)，根目录 [LICENSE](LICENSE) 为 MIT |
| GPUI / gpui_platform | [zed-industries/zed](https://github.com/zed-industries/zed)；相关 GPUI crate 声明 Apache-2.0；不把整个 Zed 项目概括为同一许可。构建时由锁定的依赖准备脚本获取，源码修补文件在 `desktop/patches` |
| GPUI Component | 依赖准备脚本记录其来源及固定提交；本版本所用 component/base/assets crate 声明 Apache-2.0，依赖自身的 LICENSE/NOTICE 保持有效 |
| Lucide 图标 | 保留 `desktop/assets/outline/LICENSE` 与来源说明 |
| Material 图标 | 保留 `desktop/assets/material/LICENSE` 与来源说明 |
| 主题调色板 | 保留 `desktop/assets/themes/SOURCES.md` 与该目录下的许可证 |
| 品牌图形 | 见 `desktop/assets/brands/SOURCES.md`，不因本项目采用 MIT 许可而视为自有品牌或获得背书 |
| KaTeX | `03_工具/学习讲义生成器/vendor/katex`；随附原始 MIT LICENSE、package.json 和来源/完整性信息；字体及构建资源保持原有许可 |
| 其他 Rust 依赖 | 由 `Cargo.lock`、`desktop/Cargo.lock` 固定；各 crate 的版权与许可证继续适用 |

本次仓库提交仅发布源码与必要的静态资源，不包含预编译的 FFmpeg、
yt-dlp、llama-server、Python、Node.js、模型权重或本机软件压缩包。
构建或运行时需另行安装的程序仍适用各自许可。

如以后发布包含这些组件的安装包，须按实际采用的版本、构建选项和分发
方式补齐许可、NOTICE 及对应源码提供等要求。这里的说明不是对已有
本地分享包完成全部第三方分发审查的声明。尤其不能把“包含一份 GPL
许可文本”当成已满足 GPL 二进制分发的全部条件。

原作者的发布脚本和安装文档保留作为源码的一部分，不表示本二次开发
版本已经发布官方安装包、获得平台签名或完成公证。
