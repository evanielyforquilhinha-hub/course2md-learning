"""Generate traceable learning notes from existing course2md artifacts, without rewriting them."""
import argparse
import base64
import hashlib
import html
import json
import mimetypes
import os
from pathlib import Path
import re
import sys
import subprocess
import time
import tomllib
import urllib.error
import urllib.request
from urllib.parse import urlsplit, urlunsplit
import webbrowser
import zipfile
from math_render import MathRenderer, strings

BASE = Path(__file__).resolve().parent
GENERATION = {'enable_thinking':True,'temperature':0.2,'max_tokens':16384}
REVIEW_RULE = '''这是学习讲义发布前的独立校核步骤。上面的图文为原始证据，下面assistant草稿不是事实依据。
请逐条核对草稿中的数学逻辑、符号、端点、单位、前提和对原图的描述。特别注意左闭右开区间的左端实心/右端空心，复数系数、正负号和分母非零；电路时钟方向是否只是示例，不把课件参数泛化。
纠正能由材料或明确推导确认的错误；不能确认的列入uncertainties，删去无根据的断言。识别错误不得说成老师口误；未显示完整的板书不得当成老师最终错误答案。
删除原材料未讲解的解析性/全纯性判别等高阶扩展；不能把“出现共轭符号”或“只含z”当成一般解析性判据。截图缺失的最后一行只能写未显示完整，禁止猜测它是1/z或推测老师写错题目。
保持43号由整体到细节的讲解与步骤理由，不压缩成提纲。检查练习答案。保持标准LaTeX公式及行内/独立公式定界符，核对上下标、共轭范围、括号、分式与积分上下限，JSON中的反斜线必须双写转义。不需要输出审查意见列表；输出修订后的完整JSON，字段结构与初稿相同。不得复制未验证错误。'''


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(path.suffix + '.tmp')
    temp.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding='utf-8')
    temp.replace(path)


class ProgressReporter:
    """Invocation-owned, atomic status; never contains prompts or credentials."""
    def __init__(self,path=None):
        self.path=path
        self.value={'schema':1,'completed':0,'total':0,'current':0,'stage':'preparing'}
        self.emit('preparing')

    def emit(self,stage,**counts):
        self.value.update(counts,stage=stage)
        if self.path:write_json(self.path,self.value)


def load_settings(path=BASE/'settings.json'):
    settings=json.loads(path.read_text(encoding='utf-8-sig'))
    config=tomllib.loads(Path(settings['config_path']).read_text(encoding='utf-8-sig'))
    settings['course_root']=config.get('defaults',{}).get('out',settings.get('course_root'))
    workspace=Path(settings['config_path']).with_name('desktop-workspace.json')
    if workspace.is_file():
        sys.path.insert(0,str(BASE.parent/'项目管理'))
        from storage import selected_course_output
        settings['course_root']=selected_course_output(workspace,settings['course_root'])
    if settings.get('storage_settings'):
        extra=json.loads(Path(settings['storage_settings']).read_text(encoding='utf-8-sig'))
        settings['outputs_root']=extra['ai_output']
    return settings


def course_folders(settings):
    roots=[Path(settings['course_root'])]
    if settings.get('history_root'):roots.append(Path(settings['history_root']))
    published={p.parent.resolve() for root in roots if root.exists() for p in root.rglob('current.json')}
    legacy={p.parent.resolve() for root in roots if root.exists() for p in root.rglob('timeline.jsonl')
            if 'versions' not in p.relative_to(root).parts
            and not any(parent in published for parent in p.parent.resolve().parents)}
    return sorted(published|legacy)


def parse_response(response):
    try:
        choice = response['choices'][0]
        if choice.get('finish_reason') != 'stop':
            raise ValueError('响应未完整结束，不能标为讲义成功：' + str(choice.get('finish_reason')))
        content = choice['message']['content']
        if not isinstance(content,str): raise ValueError('AI正文不是文本')
        fence = re.fullmatch(r'\s*```(?:json)?[ \t]*\r?\n(.*?)\r?\n```\s*',content,re.S|re.I)
        obj = json.loads(fence.group(1) if fence else content)
        def nonempty(x): return isinstance(x, str) and bool(x.strip())
        if not isinstance(obj, dict) or not all(nonempty(obj.get(k)) for k in ('title','overview')):
            raise ValueError('缺少标题或学习目标')
        sections = obj.get('sections')
        if not isinstance(sections, list) or not sections:
            raise ValueError('没有分步讲解')
        for section in sections:
            if isinstance(section,dict):
                if 'heading' not in section and 'title' in section:
                    section['heading']=section.pop('title')
                if 'paragraphs' not in section and isinstance(section.get('text'),str):
                    section['paragraphs']=[section.pop('text')]
            if not isinstance(section, dict) or not nonempty(section.get('heading')):
                raise ValueError('讲解标题无效')
            paragraphs = section.get('paragraphs')
            if isinstance(paragraphs,list):
                paragraphs=[p for p in paragraphs if not isinstance(p,str) or p.strip()]
                section['paragraphs']=paragraphs
            if not isinstance(paragraphs, list) or not paragraphs or not all(nonempty(x) for x in paragraphs):
                raise ValueError('讲解段落为空或格式无效')
        exercise = obj.get('exercise')
        if not isinstance(exercise, dict) or not all(nonempty(exercise.get(k)) for k in ('question','answer')):
            raise ValueError('练习字段不完整')
        if not isinstance(obj.get('uncertainties'), list) or not all(nonempty(x) for x in obj['uncertainties']):
            raise ValueError('待核实项格式无效')
        return obj
    except (KeyError, IndexError, TypeError, json.JSONDecodeError) as exc:
        raise ValueError('AI返回格式无效，未生成学习讲义') from exc


def fingerprint(prompt, model, unit, images):
    data = json.dumps({'prompt':prompt,'model':model,'unit':unit,
                       'images':[hashlib.sha256(b).hexdigest() for b in images]},
                      ensure_ascii=False, sort_keys=True)
    return hashlib.sha256(data.encode()).hexdigest()


def result_hash(result):
    return hashlib.sha256(json.dumps(result,ensure_ascii=False,sort_keys=True).encode()).hexdigest()


def apply_editorial(folder, uid, result):
    """Apply explicit, response-specific editor corrections; never change raw API replies."""
    path=folder/(uid+'.editorial.json')
    if not path.exists(): return result,[]
    edit=json.loads(path.read_text(encoding='utf-8'))
    if edit.get('source_sha256')!=result_hash(result):
        raise ValueError('原AI回复已变化，旧校订不能自动套用，请重新核对：'+str(path))
    fixed=json.loads(json.dumps(result));notes=[]
    for replacement in edit['replacements']:
        old,new,reason=(replacement[k] for k in ('old','new','reason'))
        if not all(isinstance(x,str) and x.strip() for x in (old,new,reason)):
            raise ValueError('校订记录字段无效')
        count=0
        def replace(value):
            nonlocal count
            if isinstance(value,str):
                count+=value.count(old)
                return value.replace(old,new)
            if isinstance(value,list):return [replace(v) for v in value]
            if isinstance(value,dict):return {k:replace(v) for k,v in value.items()}
            return value
        fixed=replace(fixed)
        if count!=1:raise ValueError('校订内容必须唯一匹配，实际次数：'+str(count))
        notes.append(reason)
    write_json(folder/(uid+'.edited.json'),{'source_sha256':result_hash(result),'result':fixed,'corrections':notes})
    return fixed,notes


def run_request(folder, uid, input_hash, body, client):
    folder.mkdir(parents=True, exist_ok=True)
    status_path = folder / f'{uid}.status.json'
    result_path = folder / f'{uid}.json'
    if status_path.exists() and result_path.exists():
        status = json.loads(status_path.read_text(encoding='utf-8'))
        if status.get('status') == 'success' and status.get('input_hash') == input_hash:
            response = json.loads(result_path.read_text(encoding='utf-8'))
            return parse_response(response), status, True
    status = {'status':'running','input_hash':input_hash,'started_at':time.strftime('%Y-%m-%d %H:%M:%S')}
    write_json(status_path, status)
    try:
        response = client(body)
        # Keep malformed/truncated provider replies for diagnosis, without request credentials.
        write_json(folder / f'{uid}.last-response.json', response)
        result = parse_response(response)
        write_json(result_path, response)
        status.update(status='success', model=response.get('model'), request_id=response.get('id'),
                      usage=response.get('usage',{}), finished_at=time.strftime('%Y-%m-%d %H:%M:%S'))
        write_json(status_path, status)
        return result, status, False
    except Exception as exc:
        status.update(status='failed', error=str(exc), finished_at=time.strftime('%Y-%m-%d %H:%M:%S'))
        write_json(status_path, status)
        raise


def safe_image(root, relative):
    p = (root / relative).resolve()
    if not p.is_relative_to(root.resolve()):
        raise ValueError('图片路径越出课程目录')
    if not p.is_file():
        raise ValueError('找不到课程截图：' + str(p))
    return p


def generate_with_review(folder, uid, input_hash, body, client, on_stage=None):
    final_path=folder/(uid+'.final.status.json')
    write_json(final_path,{'status':'running','input_hash':input_hash})
    try:
        if on_stage:on_stage('generating')
        return _generate_with_review(folder,uid,input_hash,body,client,on_stage)
    except Exception as exc:
        write_json(final_path,{'status':'failed','input_hash':input_hash,'error':str(exc)})
        raise


def _generate_with_review(folder, uid, input_hash, body, client, on_stage=None):
    draft,draft_status,draft_cached=run_request(folder,uid,input_hash,body,client)
    review_body=dict(body)
    review_body['messages']=body['messages']+[
        {'role':'assistant','content':json.dumps(draft,ensure_ascii=False)},
        {'role':'user','content':REVIEW_RULE}]
    review_hash=fingerprint(REVIEW_RULE,body.get('model',''),{'input':input_hash,'draft':draft},[])
    if on_stage:on_stage('reviewing')
    result,status,review_cached=run_request(folder,uid+'.review',review_hash,review_body,client)
    status=dict(status)
    status['draft_request_id']=draft_status.get('request_id')
    status['draft_usage']=draft_status.get('usage',{})
    status['usage']={'total_tokens':sum(s.get('usage',{}).get('total_tokens',0) for s in (draft_status,status))}
    status['review_notice']='同模型第二轮校核，并不等于人工确认正确'
    write_json(folder/(uid+'.final.status.json'),status)
    return result,status,draft_cached and review_cached


def load_course(folder):
    folder=Path(folder).resolve()
    if (folder/'current.json').is_file():
        pointer=json.loads((folder/'current.json').read_text(encoding='utf-8-sig'))
        if pointer.get('schema')!=1: raise ValueError('不支持的课程版本索引')
        folder=safe_image(folder,pointer['manifest']).parent
    if not (folder/'timeline.jsonl').is_file() and (folder/'document.json').is_file():
        document=json.loads((folder/'document.json').read_text(encoding='utf-8-sig'))
        if document.get('schema')!=1: raise ValueError('不支持的课程文档版本')
        units=[]
        for i,section in enumerate(document['sections']):
            selected=section.get('speech',[])
            if not selected: continue
            image=safe_image(folder,section['image'])
            units.append({'id':f'page-{i+1:04}','course':folder.name,
                'start':section['t'],'end':section.get('end',max(e['end'] for e in selected)),
                'images':[{'path':str(image),'t':section['t']}],
                'text':'\n'.join(f"[{e['start']:.1f}-{e['end']:.1f}] {e.get('raw') or e['text']}" for e in selected)})
        return units
    events = [json.loads(x) for x in (folder/'timeline.jsonl').read_text(encoding='utf-8').splitlines() if x.strip()]
    frames = sorted((e for e in events if e.get('type') == 'frame'), key=lambda e:e['t'])
    speech = [e for e in events if e.get('type') == 'speech']
    images = [safe_image(folder, f['image']) for f in frames]
    end = max([e['end'] for e in speech] + [f['t'] for f in frames] + [0])
    units = []
    for i,f in enumerate(frames):
        stop = frames[i+1]['t'] if i+1<len(frames) else end
        selected = [e for e in speech if e['end']>f['t'] and e['start']<stop]
        if not selected: continue
        pictures = [{'path':str(images[i]),'t':f['t']}]
        if i+1<len(frames): pictures.append({'path':str(images[i+1]),'t':frames[i+1]['t']})
        units.append({'id':f'page-{i+1:04}', 'course':folder.name,'start':f['t'],'end':stop,
                      'images':pictures, 'text':'\n'.join(f"[{e['start']:.1f}-{e['end']:.1f}] {e['text']}" for e in selected)})
    return units


def load_pilot(manifest):
    source = json.loads(manifest.read_text(encoding='utf-8'))['units']
    units=[]
    for u in source:
        events=json.loads((manifest.parent/u['transcript']).read_text(encoding='utf-8'))
        units.append({'id':u['id'],'course':u['course_id'], 'start':u['selected_range_seconds'][0],
                      'end':u['selected_range_seconds'][1],
                      'images':[{'path':str(safe_image(manifest.parent,u['image'])),'t':u['frame_time_seconds']}],
                      'text':'\n'.join(f"[{e['start']:.1f}-{e['end']:.1f}] {e['text']}" for e in events)})
    return units


def selected_llm(settings):
    if 'native_llm' in settings:
        return settings['native_llm']
    config=tomllib.loads(Path(settings['config_path']).read_text(encoding='utf-8-sig'))
    return dict(config.get('llm',{}),model=settings['model'])


def service_identity(settings):
    llm=selected_llm(settings)
    provider=llm.get('provider','openai-compatible')
    if provider=='open-ai-compatible':provider='openai-compatible'
    endpoint=llm.get('base_url','').strip().rstrip('/')
    if provider=='codex':endpoint='https://chatgpt.com/backend-api/codex'
    if endpoint:
        try:
            parsed=urlsplit(endpoint)
            if parsed.scheme.lower() not in ('http','https') or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
                raise ValueError('AI服务地址无效，不允许URL凭据或查询参数')
            host=parsed.hostname.lower()
            if ':' in host:host='['+host+']'
            if parsed.port and (parsed.scheme.lower(),parsed.port) not in (('http',80),('https',443)):host+=':'+str(parsed.port)
            endpoint=urlunsplit((parsed.scheme.lower(),host,parsed.path.rstrip('/'),'',''))
        except ValueError:raise ValueError('AI服务地址无效，不允许URL凭据或查询参数') from None
    if endpoint.endswith('/chat/completions'):endpoint=endpoint[:-len('/chat/completions')]
    return {'provider':provider,'endpoint':endpoint,'model':llm.get('model','')}


def read_native_payload(stream,require_model=True):
    raw=stream.read(1024*1024+1)
    if len(raw)>1024*1024:raise ValueError('原生AI设置超过上限')
    try: config=json.loads(raw)
    except (ValueError,TypeError):raise ValueError('原生AI设置不是有效JSON') from None
    prompt=None
    if isinstance(config,dict) and 'settings' in config:
        if set(config)-{'settings','prompt'}:raise ValueError('原生AI请求字段无效')
        prompt=config.get('prompt')
        if prompt is not None and (not isinstance(prompt,str) or not prompt.strip()):
            raise ValueError('自定义提示词必须是非空文本')
        config=config['settings']
    if not isinstance(config,dict) or config.get('provider','openai-compatible') not in ('openai-compatible','open-ai-compatible','ollama','codex'):
        raise ValueError('原生AI服务类型无效')
    for field in ('model','base_url','api_key'):
        if field in config and not isinstance(config[field],str):raise ValueError('原生AI设置字段无效')
    if require_model and not config.get('model','').strip():raise ValueError('请选择原生AI模型')
    return config,prompt


def read_native_settings(stream):
    """Compatibility helper for callers that only need the selected service."""
    return read_native_payload(stream)[0]


def selected_prompt(settings):
    prompt=settings.get('prompt_override')
    if prompt is not None:
        if not isinstance(prompt,str) or not prompt.strip():raise ValueError('自定义提示词必须是非空文本')
        return prompt
    return (BASE/'task.txt').read_text(encoding='utf-8')+'\n'+(BASE/'prompt43.txt').read_text(encoding='utf-8')


def http_client(settings):
    if 'native_llm' in settings:
        config=settings['native_llm']
        engine=os.environ.get('COURSE2MD_BIN','')
        if not engine or not Path(engine).is_file():raise ValueError('找不到原生AI引擎，请从原生课程工作台启动')
        key=config.get('api_key','')
        def call(body):
            payload={'settings':config,'messages':body['messages'],'max_tokens':body.get('max_tokens',16384)}
            try:
                completed=subprocess.run([engine,'ai-request'],input=json.dumps(payload,ensure_ascii=False),
                    text=True,encoding='utf-8',capture_output=True,timeout=360,check=False,
                    creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
            except (OSError,subprocess.SubprocessError):
                raise RuntimeError('原生AI请求未完整完成；请检查引擎和登录状态，再决定是否重试') from None
            if completed.returncode:
                # Native errors are sanitized; additionally redact the selected key.
                detail=completed.stderr.strip()[:1200]
                if key:detail=detail.replace(key,'[redacted]')
                raise RuntimeError(detail or '原生AI请求失败')
            try:return json.loads(completed.stdout)
            except ValueError:raise RuntimeError('原生AI引擎返回的协议无效，未自动重试') from None
        return call
    c=tomllib.loads(Path(settings['config_path']).read_text(encoding='utf-8'))['llm']
    key=c.get('api_key','')
    endpoint=c['base_url'].rstrip('/')
    if not endpoint.endswith('/chat/completions'): endpoint+='/chat/completions'
    if not endpoint.startswith('https://') or not key:
        raise ValueError('需要有效HTTPS服务地址和现有API密钥')
    # Never print or persist the credential; reject redirects to avoid forwarding it.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl): return None
    opener=urllib.request.build_opener(NoRedirect)
    def call(body):
        req=urllib.request.Request(endpoint, data=json.dumps(body,ensure_ascii=False).encode(),
            headers={'Authorization':'Bearer '+key,'Content-Type':'application/json'})
        try:
            with opener.open(req,timeout=180) as response: return json.load(response)
        except urllib.error.HTTPError as exc:
            detail=exc.read().decode('utf-8','replace').replace(key,'[redacted]')[:1200]
            raise RuntimeError(f'HTTP {exc.code}: {detail}') from None
        except Exception as exc:
            raise RuntimeError(str(exc).replace(key,'[redacted]')) from None
    return call


def render(folder, records, model):
    esc=lambda s:html.escape(str(s))
    math= MathRenderer(folder,list(strings([r for _,r,_ in records if r])))
    rich=math.text;mdtext=math.markdown
    pages=[]; md=['# 43号学习讲义', '', f'生成模型：{model}。AI内容需要核验；本页仅包含本次选中的单元。', '']
    for unit,result,status in records:
        uid=unit['id']; title=result['title'] if result else uid+'：生成失败'
        images=[]
        for i,img in enumerate(unit['images']):
            relative=f'assets/{uid}-{i}.jpg'
            (folder/'assets').mkdir(exist_ok=True)
            (folder/relative).write_bytes(Path(img['path']).read_bytes())
            images.append(f'<figure><a href="{relative}" target="_blank"><img src="{relative}" alt="课堂原图"></a><figcaption>原图时间 {img["t"]:.1f} 秒；点击查看大图</figcaption></figure>')
            md.extend([f'![{uid} 原图，{img["t"]:.1f}秒]({relative})',''])
        md.extend([f'## {uid} {title}', '', f'来源：{unit["course"]}，{unit["start"]:.1f}–{unit["end"]:.1f} 秒。',''])
        body=''.join(images)+f'<h2>{esc(uid)} · {rich(title) if result else esc(title)}</h2><p class="meta">{esc(unit["course"])} · {unit["start"]:.1f}–{unit["end"]:.1f} 秒</p>'
        if result:
            if status.get('editorial_notes'):
                notice='本页含逐项编辑校订；原始AI回复保留。修订：'+'；'.join(status['editorial_notes'])
                body+='<aside>'+esc(notice)+'</aside>';md.extend([notice,''])
            body+=f'<div class="prose intro">{rich(result["overview"])}</div>';md.extend([mdtext(result['overview']),''])
            for section in result['sections']:
                body+=f'<h3>{rich(section["heading"])}</h3>'
                md.extend(['### '+mdtext(section['heading']),''])
                for p in section['paragraphs']:
                    body+=f'<div class="prose">{rich(p)}</div>';md.extend([mdtext(p),''])
            q=result['exercise']
            body+=f'<h3>自测</h3><div class="prose">{rich(q["question"])}</div><details><summary>展开答案与理由</summary><div class="prose">{rich(q["answer"])}</div></details>'
            md.extend(['### 自测',mdtext(q['question']),'','<details><summary>展开答案</summary>','',mdtext(q['answer']),'','</details>',''])
            if result['uncertainties']:
                body+='<aside><h3>待核实</h3>'+''.join('<div class="prose">'+rich(s)+'</div>' for s in result['uncertainties'])+'</aside>'
                md.extend(['### 待核实',*[mdtext(s) for s in result['uncertainties']],''])
            body+=f'<p class="meta">实际模型：{esc(status.get("model",model))} · Token：{esc(status.get("usage",{}).get("total_tokens","未提供"))} · 生成成功不等于事实已核验</p>'
        else:
            body+='<aside>生成失败，未以原字幕替代学习讲义。<pre>'+esc(status['error'])+'</pre></aside>'
            md.extend(['生成失败：'+status['error'],''])
        body+='<details><summary>查看原始转录与时间</summary><pre>'+esc(unit['text'])+'</pre></details>'
        pages.append(f'<article id="{uid}">{body}</article>')
    nav=''.join(f'<a href="#{esc(u["id"])}">{esc(u["id"])}</a> ' for u,_,_ in records)
    document='''<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>43号学习讲义</title><link rel="stylesheet" href="math/katex.min.css"><style>
body{margin:0;background:#f3f4f6;color:#1e293b;font:18px/1.85 "Microsoft YaHei",sans-serif}header,main{max-width:960px;margin:auto;padding:24px}header{padding-bottom:0}nav a{display:inline-block;padding:4px 10px}article{background:white;padding:28px;margin:24px 0;border:1px solid #dce2e8;border-radius:10px}img{width:100%;height:auto}figure{margin:0 0 22px}figcaption,.meta{font-size:13px;color:#596579}p,pre{white-space:pre-wrap;overflow-wrap:anywhere}pre{font:14px/1.7 monospace}h2{font-size:27px}h3{font-size:21px;margin-top:30px}aside{background:#fff5db;padding:16px}details{padding:12px;background:#f2f5f9;margin:12px 0}summary{cursor:pointer}.intro{border-left:3px solid #2563eb;padding-left:18px}@media(max-width:650px){article{padding:16px}main,header{padding:12px}}@media print{details{display:block}article{break-before:page;border:0}nav{display:none}}</style><header><h1>43号学习讲义</h1><p>原图在上，讲解在下。来源和AI疑点保留，答案可展开。</p><nav>'''+nav+'</nav></header><main>'+''.join(pages)+'</main></html>'
    css='''.prose{margin:16px 0;white-space:pre-wrap;overflow-wrap:anywhere}.katex{font-size:1.13em;white-space:normal}.formula-block{display:block;overflow-x:auto;max-width:100%;padding:8px 4px;margin:12px 0;background:#f8fafc;border-radius:5px;white-space:normal;line-height:normal}.formula-block .katex-display{margin:8px 0}.katex-html{overflow-wrap:normal}.math-error{display:block;color:#9a3412;background:#fff1e7;padding:10px;white-space:pre-wrap}.prose.intro{padding-left:18px}@media print{.formula-block{overflow:visible;background:none}}'''
    document=document.replace('</style>','\n'+css+'</style>',1)
    if math.errors:
        document=document.replace('<nav>','<aside>有 '+str(len(math.errors))+' 处公式排版待修复，已在相应位置保留原文并提示。</aside><nav>',1)
    write_json(folder/'math-report.json',{'engine':'KaTeX local static HTML+MathML','errors':math.errors,'notice':'仅检查排版和支持的符号，不证明数学推导正确'})
    (folder/'index.html').write_text(document,encoding='utf-8')
    (folder/'学习讲义.md').write_text('\n'.join(md),encoding='utf-8')


def reformat(folder,settings):
    """Rebuild an existing output from local replies only, with no API client."""
    folder=folder.resolve()
    generation=json.loads((folder/'generation.json').read_text(encoding='utf-8'))
    if generation['source_mode']=='pilot':units=load_pilot(Path(settings['pilot_manifest']))
    else:
        dataset=folder.parent.name
        courses=course_folders(settings)
        matching=[c for c in courses if c.name==generation.get('source_course_id') or 'course-'+hashlib.sha256(str(c.resolve()).encode()).hexdigest()[:12]==dataset]
        if len(matching)!=1:raise ValueError('无法找到此学习版对应的原课程')
        units=load_course(matching[0])
    byid={u['id']:u for u in units};records=[]
    for uid in generation['selected_units']:
        status=json.loads((folder/(uid+'.final.status.json')).read_text(encoding='utf-8'))
        if status.get('status')!='success':
            records.append((byid[uid],None,{'error':status.get('error','此单元尚未成功生成')}));continue
        result=parse_response(json.loads((folder/(uid+'.review.json')).read_text(encoding='utf-8')))
        result,notes=apply_editorial(folder,uid,result);status['editorial_notes']=notes
        records.append((byid[uid],result,status))
    render(folder,records,generation['model'])
    print('已从本地结果重新排版（无API请求）：'+str(folder/'index.html'),flush=True)


def export_chatgpt(folder,units,prompt,prompt_source='default'):
    """Package evidence for an explicit manual upload. This path has no AI client."""
    folder=Path(folder).resolve();folder.mkdir(parents=True,exist_ok=True)
    (folder/'images').mkdir(exist_ok=True)
    esc=lambda value:html.escape(str(value),quote=True)
    math=MathRenderer(folder,[u['text'] for u in units])
    files=[];transcript=[];pages=[];image_count=0;image_size=0
    for index,unit in enumerate(units,1):
        figures=[];references=[]
        for image_index,img in enumerate(unit['images'],1):
            source=Path(img['path']).resolve()
            if not source.is_file() or source.suffix.lower() not in ('.jpg','.jpeg','.png','.webp','.gif'):
                raise ValueError('导出材料包含无效图片')
            name=f'images/unit-{index:04}-{image_index:02}{source.suffix.lower()}'
            data=source.read_bytes();(folder/name).write_bytes(data)
            files.append(folder/name);image_count+=1;image_size+=len(data)
            references.append(f'{name}（{img["t"]:.1f} 秒）')
            figures.append(f'<figure><img src="{name}" alt="课程截图"><figcaption>{esc(references[-1])}</figcaption></figure>')
        title=f'{unit["id"]} · {unit["course"]} · {unit["start"]:.1f}–{unit["end"]:.1f} 秒'
        transcript.append(title+'\n截图：'+'；'.join(references)+'\n'+unit['text'])
        pages.append('<article><h2>'+esc(title)+'</h2>'+''.join(figures)+'<div class="source">'+math.text(unit['text'])+'</div></article>')
    instructions=(f'ChatGPT 网页手动上传材料包\n\n本包包含 {len(units)} 个图文单元、{image_count} 张图片，'
        f'图片合计 {image_size/1024/1024:.2f} MiB；首轮上传文件为 {image_count+1} 个（图片及转录），另附完整提示词。\n'
        '1. 自行打开 ChatGPT 网页并新建对话，选择支持图片的模型。\n'
        '2. 解压 ZIP；上传 images 中的图片及 transcript.txt，将 prompt43.txt 的完整内容粘贴为指令。\n'
        '3. 按文件名核对截图与转录对应关系，要求按43号提示词生成完整讲义。\n'
        '4. 若网页提示文件数量或大小超限，请按单元分批上传；每批附对应转录及完整提示词。以网页实际提示为准。\n'
        '5. 生成后可再发送 review.txt，要求基于原始截图和转录核对草稿。\n\n'
        'index.html 仅供本地预览材料。本程序没有连接 ChatGPT 网页，也没有提交AI请求或生成AI讲义；上传和对话须由你完成。\n')
    for name,text in [('README-上传说明.txt',instructions),('prompt43.txt',prompt),('prompt-used.txt',prompt),('transcript.txt','\n\n'.join(transcript)),('review.txt',REVIEW_RULE)]:
        path=folder/name
        with path.open('w',encoding='utf-8',newline='\n') as stream:
            stream.write(text)
        files.append(path)
    manifest=folder/'export.json'
    write_json(manifest,{'mode':'manual-chatgpt-export','prompt_sha256':hashlib.sha256(prompt.encode()).hexdigest(),
        'prompt_source':prompt_source,'selected_units':[u['id'] for u in units],'image_count':image_count,'image_bytes':image_size,
        'notice':'仅导出本地材料，未连接ChatGPT网页或提交AI请求'})
    files.append(manifest)
    preview='''<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>ChatGPT 手动上传材料预览</title><link rel="stylesheet" href="math/katex.min.css"><style>body{max-width:960px;margin:24px auto;padding:20px;font:17px/1.8 sans-serif;background:#f3f4f6;color:#1e293b}article{padding:24px;margin:24px 0;background:white;border:1px solid #dce2e8}img{max-width:100%;height:auto}pre,.source{white-space:pre-wrap;overflow-wrap:anywhere}figure{margin:20px 0}.formula-block{overflow:auto}figcaption{font-size:13px}</style><h1>ChatGPT 手动上传材料预览</h1><pre>'''+esc(instructions)+'</pre>'+''.join(pages)+'</html>'
    index=folder/'index.html';index.write_text(preview,encoding='utf-8');files.append(index)
    if (folder/'math').is_dir():files.extend(p for p in (folder/'math').rglob('*') if p.is_file())
    archive=folder/'ChatGPT-手动上传材料.zip'
    with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_DEFLATED) as bundle:
        for path in files:bundle.write(path,path.relative_to(folder).as_posix())
    return {'status':'success','output_html':str(index),'export_zip':str(archive),'selected_units':[u['id'] for u in units],
        'image_count':image_count,'image_bytes':image_size,'error':None,'mode':'manual-chatgpt-export'}


def describe(settings,course=None):
    llm=selected_llm(settings)
    prompt=selected_prompt(settings)
    return {'model':llm.get('model',''),'provider':llm.get('provider','openai-compatible'),'base_url':llm.get('base_url',''),
            'api_key_present':bool(llm.get('api_key')),
            'auth_configured':bool(llm.get('enabled')) if llm.get('provider')=='codex' else bool(llm.get('api_key')) or llm.get('provider')=='ollama',
            'unit_count':len(load_course(course.resolve())) if course else 0,
            'outputs_root':settings.get('outputs_root',str(BASE/'outputs')),
            'prompt_sha256':hashlib.sha256(prompt.encode()).hexdigest(),
            'prompt_source':'custom' if settings.get('prompt_override') is not None else 'default'}


def main(argv=None):
    if hasattr(sys.stdout,'reconfigure'): sys.stdout.reconfigure(encoding='utf-8')
    ap=argparse.ArgumentParser(description='按43号提示词，将已有课程图文转成学习讲义')
    ap.add_argument('--course',type=Path);ap.add_argument('--pilot',action='store_true')
    ap.add_argument('--ids',help='仅基准集使用，例如 C03,M02,E03')
    ap.add_argument('--limit',default='3',help='本次单元数，默认3；all表示全部')
    ap.add_argument('--interactive',action='store_true');ap.add_argument('--open',action='store_true')
    ap.add_argument('--reformat',type=Path,help='只重新排版已有输出目录，不调用AI')
    ap.add_argument('--describe',action='store_true',help='仅向原生界面返回真实配置和单元数，不调用AI')
    ap.add_argument('--native-ai-stdin',action='store_true',help='从标准输入读取原生AI设置，或包含settings和可选prompt的JSON对象')
    ap.add_argument('--export-chatgpt',action='store_true',help='离线导出ChatGPT网页手动上传材料包')
    ap.add_argument('--result-file',type=Path,help='原生界面使用的任务结果JSON')
    ap.add_argument('--progress-file',type=Path,help='原生界面使用的实时进度JSON')
    args=ap.parse_args(argv)
    progress=ProgressReporter(args.progress_file)
    try:
        settings=load_settings()
        if args.native_ai_stdin:
            settings['native_llm'],settings['prompt_override']=read_native_payload(sys.stdin,require_model=not (args.export_chatgpt or args.describe))
            settings['model']=settings['native_llm'].get('model','')
        return run(args,settings,progress)
    except Exception as exc:
        progress.emit('failed')
        if args.result_file:
            write_json(args.result_file,{'status':'failed','output_html':None,'selected_units':[],'error':str(exc)})
        raise


def run(args,settings,progress=None):
    progress=progress or ProgressReporter(getattr(args,'progress_file',None))
    if args.describe:
        print(json.dumps(describe(settings,args.course),ensure_ascii=False));return 0
    if args.reformat:
        reformat(args.reformat,settings)
        if args.open:webbrowser.open((args.reformat.resolve()/'index.html').as_uri())
        return 0
    if args.interactive:
        folders=course_folders(settings)
        print('选择已转录课程（原稿不变，学习版另存）：')
        for i,p in enumerate(folders,1): print(f'{i}. {p.parent.name}')
        print('0. 十二单元校核基准集')
        answer=input('课程编号：').strip()
        if answer=='0': args.pilot=True
        elif answer.isdigit() and 1<=int(answer)<=len(folders): args.course=folders[int(answer)-1]
        else: raise ValueError('课程编号无效')
        args.limit=input('本次处理多少页？回车=3，all=全部（在线生成会使用所选AI服务）：').strip() or '3'
    if args.pilot:
        units=load_pilot(Path(settings['pilot_manifest']));dataset='pilot12'
    elif args.course:
        units=load_course(args.course.resolve());dataset='course-'+hashlib.sha256(str(args.course.resolve()).encode()).hexdigest()[:12]
    else: raise ValueError('请选择 --course 路径、--pilot 或 --interactive')
    if args.ids:
        chosen=set(args.ids.split(','));units=[u for u in units if u['id'] in chosen]
        if {u['id'] for u in units}!=chosen: raise ValueError('指定单元不存在')
    if args.limit!='all':
        n=int(args.limit)
        if n<1: raise ValueError('数量至少为1')
        units=units[:n]
    if not units: raise ValueError('没有可处理的图文单元')
    progress.emit('preparing',total=len(units))
    prompt=selected_prompt(settings)
    prompt_source='custom' if settings.get('prompt_override') is not None else 'default'
    if args.export_chatgpt:
        revision=hashlib.sha256((prompt+json.dumps(units,sort_keys=True,ensure_ascii=False)).encode()).hexdigest()[:12]
        folder=Path(settings.get('outputs_root',str(BASE/'outputs')))/dataset/('chatgpt-export-'+revision)
        result=export_chatgpt(folder,units,prompt,prompt_source)
        if args.result_file:write_json(args.result_file,result)
        progress.emit('complete',completed=len(units),current=len(units))
        print('ChatGPT手动上传材料包：'+result['export_zip'],flush=True)
        if args.open:webbrowser.open(Path(result['output_html']).as_uri())
        return 0
    identity=service_identity(settings)
    model=identity['model'];client=http_client(settings)
    revision=hashlib.sha256((prompt+json.dumps(identity,sort_keys=True)+json.dumps(GENERATION,sort_keys=True)).encode()).hexdigest()[:12]
    folder=Path(settings.get('outputs_root',str(BASE/'outputs')))/dataset/revision;folder.mkdir(parents=True,exist_ok=True)
    write_json(folder/'generation.json',{'model':model,'service':identity,'prompt_sha256':hashlib.sha256(prompt.encode()).hexdigest(),'prompt_source':prompt_source,
        'selected_units':[u['id'] for u in units],'generation_settings':GENERATION,'source_mode':'pilot' if args.pilot else 'existing-course-frames',
        'review_rule_sha256':hashlib.sha256(REVIEW_RULE.encode()).hexdigest(),
        'source_course_id':args.course.resolve().name if args.course else None,
        'notice':'原图逐段关联；没有自动语义去重；成功表示生成和同模型第二轮校核通过，不保证事实正确'})
    with (folder/'prompt-used.txt').open('w',encoding='utf-8',newline='\n') as stream:
        stream.write(prompt)
    records=[];failed=False
    for i,unit in enumerate(units,1):
        progress.emit('generating',completed=i-1,current=i)
        print(f'[{i}/{len(units)}] {unit["id"]} → {model}，读取图片与转录...',flush=True)
        image_bytes=[Path(img['path']).read_bytes() for img in unit['images']]
        h=fingerprint(prompt,model,{'unit':unit,'generation':GENERATION,'service':identity},image_bytes)
        text=json.dumps({k:v for k,v in unit.items() if k!='images'},ensure_ascii=False)
        content=[{'type':'text','text':text}]
        for img,data in zip(unit['images'],image_bytes):
            content.append({'type':'text','text':f'下面是视频 {img["t"]:.1f} 秒的截图。'})
            mime=mimetypes.guess_type(img['path'])[0] or 'image/jpeg'
            content.append({'type':'image_url','image_url':{'url':f'data:{mime};base64,'+base64.b64encode(data).decode()}})
        body={'model':model,'messages':[{'role':'system','content':prompt},{'role':'user','content':content}],
              **GENERATION}
        try:
            result,status,cached=generate_with_review(folder,unit['id'],h,body,client,progress.emit)
            result,notes=apply_editorial(folder,unit['id'],result)
            status['editorial_notes']=notes
            print(('复用已完成结果' if cached else '实际调用成功')+f'；tokens={status.get("usage",{}).get("total_tokens","未知")}',flush=True)
            records.append((unit,result,status))
        except Exception as exc:
            records.append((unit,None,{'error':str(exc)}));failed=True
            print('失败：'+str(exc),flush=True)
        progress.emit('rendering')
        render(folder,records,model)
        if failed:
            progress.emit('failed')
            print('已停止后续调用。修复后重运行，成功单元会复用，避免重复计费。');break
    print('学习版：'+str(folder/'index.html'),flush=True)
    if args.result_file:
        write_json(args.result_file,{'status':'failed' if failed else 'success',
            'output_html':str(folder/'index.html'),'selected_units':[u['id'] for u,_,_ in records],
            'error':records[-1][2].get('error') if failed else None})
    if not failed:progress.emit('complete',completed=len(units),current=len(units))
    if args.open or args.interactive: webbrowser.open((folder/'index.html').as_uri())
    return 1 if failed else 0


if __name__=='__main__':
    try: sys.exit(main())
    except (ValueError,OSError,KeyError) as exc:
        print('无法生成：'+str(exc),file=sys.stderr);sys.exit(1)
