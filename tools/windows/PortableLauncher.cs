using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Web.Script.Serialization;
using System.Windows.Forms;

internal static class PortableLauncher {
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    static readonly UTF8Encoding Utf8 = new UTF8Encoding(false);
    static string At(string root, string path) { return Path.Combine(root, path); }
    static void Save(string path, object value) {
        Directory.CreateDirectory(Path.GetDirectoryName(path));
        string temp = path + ".new";
        File.WriteAllText(temp, Json.Serialize(value), Utf8);
        if (File.Exists(path)) File.Replace(temp, path, null);
        else File.Move(temp, path);
    }
    static Dictionary<string,object> Read(string path) {
        return File.Exists(path) ? Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(path, Utf8)) : new Dictionary<string,object>();
    }
    static void Initialize(string root) {
        string config = At(root, @"04_配置\course2md\config.toml");
        string storage = At(root, @"04_配置\storage.json");
        string settingsFile = At(root, @"03_工具\学习讲义生成器\settings.json");
        foreach (string path in new string[] { "04_配置", "05_课程资料", "06_学习讲义", @"07_模型与缓存\cache", @"07_模型与缓存\临时文件", @"07_模型与缓存\运行日志" }) Directory.CreateDirectory(At(root,path));
        Directory.CreateDirectory(Path.GetDirectoryName(config));
        if (!File.Exists(config)) File.WriteAllText(config, "# Portable defaults; AI services are configured in the application.\n[defaults]\nout = " + Json.Serialize(At(root,"05_课程资料")) + "\n", Utf8);
        var settings = Read(settingsFile);
        object previous;
        string oldRoot = settings.TryGetValue("portable_root", out previous) ? previous as string : null;
        var locations = Read(storage);
        object output;
        if (!locations.TryGetValue("ai_output", out output) || !(output is string)) locations["ai_output"] = At(root,"06_学习讲义");
        else if (!String.IsNullOrEmpty(oldRoot) && !String.Equals(oldRoot,root,StringComparison.OrdinalIgnoreCase)) {
            string oldOutput = (string)output;
            if (oldOutput.StartsWith(oldRoot + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase)) locations["ai_output"] = root + oldOutput.Substring(oldRoot.Length);
        }
        Save(storage, locations);
        settings["portable_root"] = root;
        settings["config_path"] = config;
        settings["course_root"] = At(root,"05_课程资料");
        settings["history_root"] = At(root,@"05_课程资料\历史课程");
        settings["outputs_root"] = locations["ai_output"];
        settings["storage_settings"] = storage;
        settings["model"] = "";
        Save(settingsFile,settings);
    }
    [STAThread] private static int Main(string[] args) {
        bool initializeOnly = args.Length == 1 && args[0] == "--initialize-only";
        try {
            string root = AppDomain.CurrentDomain.BaseDirectory.TrimEnd(Path.DirectorySeparatorChar);
            Initialize(root);
            if (initializeOnly) return 0;
            if (args.Length == 1 && args[0] == "--choose-output") {
                string storage = At(root,@"04_配置\storage.json");
                var locations = Read(storage);
                using (var picker = new FolderBrowserDialog()) {
                    picker.Description = "选择 AI 学习讲义的保存文件夹；已有文件不会自动搬动";
                    picker.SelectedPath = (string)locations["ai_output"];
                    if (picker.ShowDialog() == DialogResult.OK) { locations["ai_output"] = picker.SelectedPath; Save(storage,locations); }
                }
                return 0;
            }
            string binary = At(root,@"01_软件\当前版\course2md-desktop.exe");
            if (!File.Exists(binary)) throw new FileNotFoundException("请先完整解压分享包，再运行启动器。", binary);
            var info = new ProcessStartInfo(binary) { UseShellExecute = false, WorkingDirectory = At(root,@"07_模型与缓存\运行日志") };
            info.EnvironmentVariables["XDG_CONFIG_HOME"] = At(root,"04_配置");
            info.EnvironmentVariables["XDG_CACHE_HOME"] = At(root,@"07_模型与缓存\cache");
            info.EnvironmentVariables["TEMP"] = info.EnvironmentVariables["TMP"] = At(root,@"07_模型与缓存\临时文件");
            info.EnvironmentVariables["HF_HOME"] = At(root,@"07_模型与缓存\huggingface");
            info.EnvironmentVariables["CUDA_CACHE_PATH"] = At(root,@"07_模型与缓存\gpu");
            info.EnvironmentVariables["COURSE2MD_BIN"] = At(root,@"01_软件\当前版\course2md.exe");
            info.EnvironmentVariables["COURSE2MD_YTDLP"] = At(root,@"01_软件\course2md\runtime\media\yt-dlp.exe");
            info.EnvironmentVariables["COURSE2MD_STUDY_SCRIPT"] = At(root,@"03_工具\学习讲义生成器\study.py");
            info.EnvironmentVariables["COURSE2MD_STUDY_PYTHON"] = At(root,@"03_工具\runtime\python\python.exe");
            info.EnvironmentVariables["PATH"] = At(root,@"01_软件\course2md\runtime\media") + ";" + At(root,@"01_软件\course2md\runtime\llama") + ";" + info.EnvironmentVariables["PATH"];
            info.EnvironmentVariables.Remove("COURSE2MD_VALIDATION_WINDOW");
            Process.Start(info);
            return 0;
        } catch (Exception error) {
            if (!initializeOnly) MessageBox.Show(error.Message,"课程学习助手",MessageBoxButtons.OK,MessageBoxIcon.Error);
            return 1;
        }
    }
}
