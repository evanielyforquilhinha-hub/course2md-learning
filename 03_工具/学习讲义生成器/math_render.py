"""Compile delimited math locally. Legacy formatting never rewrites algebra."""
import html
import json
from pathlib import Path
import re
import shutil
import subprocess

BASE=Path(__file__).resolve().parent
NODE=BASE.parent/'runtime/node/node.exe'
OPEN=re.compile(r'\$\$|\\\[|\\\(|(?<!\\)\$')
LEGACY=re.compile(r'[A-Za-z0-9α-ωΑ-Ω₀-₉⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻₊₋\u0304∫∑∞√≤≥≠±·×−=+*/^_()\[\]{},.|<> \t-]+')
SUPER=str.maketrans('⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻','0123456789+-')
SUB=str.maketrans('₀₁₂₃₄₅₆₇₈₉₊₋','0123456789+-')
CONTROL=re.compile(r'[\x00-\x08\x0b\x0c\x0e-\x1f\ufffd]')


def visible_source(text):
    return CONTROL.sub(lambda m:'[U+'+f'{ord(m[0]):04X}'+']',text)


def legacy_tex(source):
    text=re.sub(r'([A-Za-z])\u0304',r'\\overline{\1}',source)
    text=re.sub(r'[⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻]+',lambda m:'^{'+m[0].translate(SUPER)+'}',text)
    text=re.sub(r'[₀-₉₊₋]+',lambda m:'_{'+m[0].translate(SUB)+'}',text)
    text=re.sub(r'\b(Re|Im)\b',r'\\operatorname{\1}',text)
    text=re.sub(r'√\(([^()]*)\)',r'\\sqrt{(\1)}',text)
    return text


def pieces(source):
    """Return text/math/error spans without interpreting ambiguous slash notation."""
    def legacy(text):
        if re.search(r'\\[A-Za-z]+',text):
            yield ('error',text,False);return
        pos=0
        for m in LEGACY.finditer(text):
            if m.start()>pos:yield ('text',text[pos:m.start()],False)
            raw=m[0]
            # Plain English words and prose punctuation are not mathematical formulas.
            if re.search(r'[=₀-₉⁰¹²³⁴⁵⁶⁷⁸⁹\u0304≤≥≠√∫∑]',raw) and re.search(r'[A-Za-zα-ωΑ-Ω]',raw):
                leading=raw[:len(raw)-len(raw.lstrip())];trailing=raw[len(raw.rstrip()):]
                if leading:yield ('text',leading,False)
                yield ('math',legacy_tex(raw.strip()),False)
                if trailing:yield ('text',trailing,False)
            else:yield ('text',raw,False)
            pos=m.end()
        if pos<len(text):yield ('text',text[pos:],False)
    pos=0
    while match:=OPEN.search(source,pos):
        yield from legacy(source[pos:match.start()])
        opening=match[0];closing={'\\[':'\\]','\\(':'\\)'}.get(opening,opening)
        end=source.find(closing,match.end())
        if end<0:
            yield ('error',source[match.start():],False);return
        tex=source[match.end():end]
        yield ('math',tex,opening in ('$$','\\['))
        pos=end+len(closing)
    yield from legacy(source[pos:])


def strings(value):
    if isinstance(value,str):yield value
    elif isinstance(value,dict):
        for v in value.values():yield from strings(v)
    elif isinstance(value,(list,tuple)):
        for v in value:yield from strings(v)


class MathRenderer:
    def __init__(self,folder,texts):
        self.errors=[];self.output={};self.parts={}
        requests=[];keys=[]
        for source in dict.fromkeys(texts):
            if CONTROL.search(source):
                spans=[('error',source,False)]
            else:spans=list(pieces(source))
            self.parts[source]=spans
            for kind,tex,display in spans:
                key=(tex,display)
                if kind=='math' and key not in keys:
                    keys.append(key);requests.append({'tex':tex,'display':display})
        if requests:
            node=str(NODE) if NODE.is_file() else shutil.which('node')
            if not node:raise ValueError('本地公式排版需要Node运行时，未找到可用程序')
            run=subprocess.run([node,str(BASE/'math-render.cjs')],input=json.dumps(requests,ensure_ascii=False),
                text=True,encoding='utf-8',capture_output=True,timeout=45,
                creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
            if run.returncode:raise ValueError('公式排版程序失败：'+run.stderr[:500])
            replies=json.loads(run.stdout)
            if len(replies)!=len(keys):raise ValueError('公式排版结果数量不一致')
            self.output=dict(zip(keys,replies))
        target=folder/'math';target.mkdir(parents=True,exist_ok=True)
        vendor=BASE/'vendor/katex'
        shutil.copy2(vendor/'dist/katex.min.css',target/'katex.min.css')
        shutil.copytree(vendor/'dist/fonts',target/'fonts',dirs_exist_ok=True)
        shutil.copy2(vendor/'LICENSE',target/'LICENSE')
        for source in self.parts:
            for kind,tex,display in self.parts[source]:
                error='公式定界符未闭合或含异常字符' if kind=='error' else self.output.get((tex,display),{}).get('error')
                if error:self.errors.append({'source':source,'formula':tex,'error':error})

    def text(self,source):
        out=[]
        for kind,tex,display in self.parts[source]:
            if kind=='text':out.append(html.escape(tex));continue
            result=self.output.get((tex,display),{})
            if kind=='error' or result.get('error'):
                # Keep source visible, including invalid JSON escape control characters.
                visible=visible_source(tex)
                out.append('<span class="math-error">公式排版待修复：<code>'+html.escape(visible)+'</code></span>')
            else:
                out.append('<span class="formula-block">'+result['html']+'</span>' if display else result['html'])
        return ''.join(out)

    def markdown(self,source):
        out=[]
        for kind,tex,display in self.parts[source]:
            if kind=='text':out.append(tex)
            elif kind=='error' or self.output.get((tex,display),{}).get('error'):out.append('【公式排版待修复】'+visible_source(tex))
            else:out.append(('$$'+tex+'$$') if display else ('$'+tex+'$'))
        return ''.join(out)
