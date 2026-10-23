"""One authoritative configuration and project-local launch environment."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import tomllib

ROOT=Path(__file__).resolve().parents[2]


def atomic_text(path,text):
    path.parent.mkdir(parents=True,exist_ok=True)
    temp=path.with_suffix(path.suffix+'.tmp')
    temp.write_text(text,encoding='utf-8');temp.replace(path)


def atomic_json(path,data):
    atomic_text(path,json.dumps(data,ensure_ascii=False,indent=2))


def config_path(root):return root/'04_配置/course2md/config.toml'


def selected_course_output(workspace,fallback):
    if not workspace.is_file():return fallback
    state=json.loads(workspace.read_text(encoding='utf-8-sig'))
    if state.get('schema')!=1:raise ValueError('新版工作区格式不受支持，请在软件内管理笔记库')
    return next(library['root'] for library in state['libraries'] if library['id']==state['default_library'])


def get_paths(root=ROOT):
    workspace=config_path(root).with_name('desktop-workspace.json')
    fallback=None if workspace.is_file() else tomllib.loads(config_path(root).read_text(encoding='utf-8-sig'))['defaults']['out']
    extra=json.loads((root/'04_配置/storage.json').read_text(encoding='utf-8-sig'))
    return {'course_output':selected_course_output(workspace,fallback),'ai_output':extra['ai_output'],
            'course_managed_by_native':workspace.is_file(),
            'history_root':str(root/'05_课程资料/历史课程'),
            'cache_root':str(root/'07_模型与缓存/cache')}


def writable_folder(value):
    p=Path(value).expanduser()
    if not value.strip() or not p.is_absolute():raise ValueError('请选择完整的绝对文件夹路径')
    p=p.resolve();p.mkdir(parents=True,exist_ok=True)
    with tempfile.NamedTemporaryFile(prefix='.course-write-check-',dir=p):pass
    return p


def update_default(text,key,value):
    """Update a single TOML defaults key, preserving credentials and other settings."""
    section=re.search(r'^\[defaults\][ \t]*\r?\n(?P<body>.*?)(?=^\[|\Z)',text,re.M|re.S)
    if not section:raise ValueError('原配置缺少defaults分区，不能安全更新')
    body=section['body'];line=key+' = '+json.dumps(str(value),ensure_ascii=False)+'\n'
    pattern=r'^'+re.escape(key)+r'[ \t]*=.*(?:\n|$)'
    body=re.sub(pattern,lambda _:line,body,count=1,flags=re.M) if re.search(pattern,body,re.M) else line+body
    new=text[:section.start('body')]+body+text[section.end('body'):]
    tomllib.loads(new)
    return new


def save_paths(root,course_output,ai_output):
    current=get_paths(root)
    if current['course_managed_by_native'] and Path(course_output).resolve()!=Path(current['course_output']).resolve():
        raise ValueError('新版课程位置请在软件设置的笔记库管理中调整；此入口仍可设置AI讲义位置')
    # Validate every destination before changing either configuration file.
    course=writable_folder(course_output);ai=writable_folder(ai_output)
    cfg_path=config_path(root);extra_path=root/'04_配置/storage.json'
    old_cfg=cfg_path.read_text(encoding='utf-8-sig');old_extra=extra_path.read_text(encoding='utf-8-sig')
    extra=json.loads(old_extra);extra['ai_output']=str(ai)
    new_cfg=update_default(old_cfg,'out',course)
    backup=cfg_path.with_name('config.toml.before-path-change-'+str(time.time_ns())+'.bak')
    shutil.copy2(cfg_path,backup)
    try:
        atomic_text(cfg_path,new_cfg);atomic_json(extra_path,extra)
    except Exception:
        atomic_text(cfg_path,old_cfg);atomic_text(extra_path,old_extra);raise
    return get_paths(root)


def launch_environment(root=ROOT,environ=None):
    env=dict(os.environ if environ is None else environ)
    env['XDG_CONFIG_HOME']=str(root/'04_配置')
    env['XDG_CACHE_HOME']=str(root/'07_模型与缓存/cache')
    env['HF_HOME']=str(root/'07_模型与缓存/huggingface')
    env['CUDA_CACHE_PATH']=str(root/'07_模型与缓存/gpu')
    env['TEMP']=env['TMP']=str(root/'07_模型与缓存/临时文件')
    env['PYTHONPYCACHEPREFIX']=str(root/'07_模型与缓存/python-cache')
    binary=next((root/'01_软件'/name for name in ('当前版','定制版','course2md') if (root/'01_软件'/name/'course2md-desktop.exe').is_file()),root/'01_软件/course2md')
    env['COURSE2MD_BIN']=str(binary/'course2md.exe')
    env['COURSE2MD_STUDY_SCRIPT']=str(root/'03_工具/学习讲义生成器/study.py')
    env['COURSE2MD_STUDY_PYTHON']=str(root/'03_工具/runtime/python/python.exe')
    env['COURSE2MD_YTDLP']=str(root/'01_软件/course2md/runtime/media/yt-dlp.exe')
    env['PATH']=os.pathsep.join([str(root/'01_软件/course2md/runtime/media'),str(root/'01_软件/course2md/runtime/llama'),env.get('PATH','')])
    return env


def launch_native(root=ROOT):
    env=launch_environment(root)
    for key in ('TEMP','HF_HOME','CUDA_CACHE_PATH','PYTHONPYCACHEPREFIX'):
        Path(env[key]).mkdir(parents=True,exist_ok=True)
    get_paths(root) # Fail before launching if the authoritative config is malformed.
    exe=Path(env['COURSE2MD_BIN']).with_name('course2md-desktop.exe')
    logs=root/'07_模型与缓存/运行日志';logs.mkdir(parents=True,exist_ok=True)
    with (logs/'desktop.log').open('ab') as log:
        return subprocess.Popen([str(exe)],cwd=logs,env=env,stdin=subprocess.DEVNULL,
            stdout=log,stderr=subprocess.STDOUT,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument('--get',action='store_true');ap.add_argument('--save-json',type=Path)
    ap.add_argument('--settings',action='store_true');ap.add_argument('--doctor',action='store_true')
    args=ap.parse_args()
    if args.get:print(json.dumps(get_paths(),ensure_ascii=False));return
    if args.save_json:
        data=json.loads(args.save_json.read_text(encoding='utf-8-sig'))
        print(json.dumps(save_paths(ROOT,data['course_output'],data['ai_output']),ensure_ascii=False));return
    if args.settings:
        subprocess.Popen(['powershell.exe','-NoProfile','-STA','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File',str(Path(__file__).with_name('保存位置设置.ps1'))],creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0));return
    if args.doctor:
        env=launch_environment()
        exe=Path(env['COURSE2MD_BIN'])
        for key in ('TEMP','HF_HOME','CUDA_CACHE_PATH','PYTHONPYCACHEPREFIX'):
            Path(env[key]).mkdir(parents=True,exist_ok=True)
        completed=subprocess.run([str(exe),'doctor'],cwd=exe.parent,env=env,capture_output=True,text=True,encoding='utf-8',errors='replace')
        print(completed.stdout);print(completed.stderr);raise SystemExit(completed.returncode)
    launch_native()


if __name__=='__main__':
    try:main()
    except Exception as exc:
        message='课程助手操作失败：'+str(exc)
        if os.name=='nt':
            import ctypes
            ctypes.windll.user32.MessageBoxW(None,message,'课程学习助手',16)
        else:print(message)
        raise SystemExit(1)
