# 学习讲义生成工具

Python 3.11+、Node.js（建议 18+），公式排版资源 KaTeX 已随源码保留其
许可证。调用云端 AI 还需要用户在桌面程序内配置自己的服务或账号。

`study.py` 读取已有课程截图和转录，生成独立 HTML/Markdown 讲义，保留
课堂笔记。`task.txt` 是批处理和数学格式适配要求，`prompt43.txt` 是为
旧版文件接口保留的兼容名称，内容是通用教学模板；不包含私人提示词。
软件的默认教学提示词与本次课程提示词均可修改。

将 `settings.example.json` 复制为 `settings.json` 后替换示例路径，或按
[开发文档](../../docs/DEVELOPMENT.md)中的 PowerShell 说明生成。`settings.json`、输出和个人
配置均不提交。不要在示例中填入真实 API Key。

桌面程序通过 `COURSE2MD_STUDY_SCRIPT` 和 `COURSE2MD_STUDY_PYTHON` 定位
工具，并经标准输入传入本次配置；`COURSE2MD_BIN` 指向配套转换引擎。
`--native-ai-stdin` 不从示例配置替换本次所选模型。

离线测试（使用合成材料及模拟 AI 响应，不发送付费模型请求）：

```powershell
python -X utf8 -B -m unittest discover -s .\03_工具\学习讲义生成器 -p "test_*.py"
```

只导出材料到 ChatGPT 的模式不会自动调用网页或上传。自动模式的额度
和费用取决于用户选择的服务；普通摘要、字幕校对和教学讲义为独立选项。
