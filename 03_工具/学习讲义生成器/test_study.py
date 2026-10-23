import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import io
import zipfile
import sys


class StudyTests(unittest.TestCase):
    def test_progress_file_tracks_actual_generation_review_and_rendering(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);course=root/'course';course.mkdir();(course/'frame.png').write_bytes(b'image')
            self.s.write_json(course/'document.json',{'schema':1,'sections':[
                {'t':i,'end':i+1,'image':'frame.png','speech':[{'start':i,'end':i+1,'text':'材料'}]} for i in range(2)]})
            progress=root/'progress.json';result=root/'result.json';seen=[]
            def client(body):
                seen.append(json.loads(progress.read_text(encoding='utf-8')))
                return self.response()
            def render(folder,records,model):
                meter=json.loads(progress.read_text(encoding='utf-8'))
                self.assertEqual(meter['stage'],'rendering')
                self.assertEqual(meter['completed'],len(records)-1)
                (folder/'index.html').write_text('local fixture',encoding='utf-8')
            with patch.object(self.s,'load_settings',return_value={'outputs_root':str(root/'outputs'),'native_llm':{'provider':'codex','model':'fixture','api_key':'private'}}), patch.object(self.s,'http_client',return_value=client), patch.object(self.s,'render',side_effect=render), patch.object(self.s.sys,'stdout',io.StringIO()):
                self.assertEqual(self.s.main(['--course',str(course),'--limit','all','--progress-file',str(progress),'--result-file',str(result)]),0)
            self.assertEqual([(m['completed'],m['current'],m['stage']) for m in seen],[(0,1,'generating'),(0,1,'reviewing'),(1,2,'generating'),(1,2,'reviewing')])
            final=json.loads(progress.read_text(encoding='utf-8'))
            self.assertEqual((final['schema'],final['completed'],final['total'],final['stage']),(1,2,2,'complete'))
            self.assertNotIn('private',progress.read_text(encoding='utf-8'))

    def test_native_config_is_authoritative_and_cache_identity_excludes_key(self):
        native={'provider':'codex','model':'selected-model','base_url':'','api_key':'secret','enabled':True}
        settings={'model':'old-qwen','native_llm':native,'config_path':'missing'}
        data=self.s.describe(settings)
        self.assertEqual((data['model'],data['provider']),('selected-model','codex'))
        self.assertTrue(data['auth_configured'])
        identity=self.s.service_identity(settings)
        self.assertNotIn('secret',json.dumps(identity))
        other={'native_llm':dict(native,api_key='other')}
        self.assertEqual(identity,self.s.service_identity(other))
        self.assertNotEqual(identity,self.s.service_identity({'native_llm':dict(native,provider='ollama',base_url='http://localhost:11434/v1')}))

    def test_failed_review_keeps_completed_count_and_retry_reuses_successful_work(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);course=root/'course';course.mkdir();(course/'frame.png').write_bytes(b'image')
            self.s.write_json(course/'document.json',{'schema':1,'sections':[
                {'t':i,'end':i+1,'image':'frame.png','speech':[{'start':i,'end':i+1,'text':'材料'}]} for i in range(2)]})
            progress=root/'progress.json';result=root/'result.json';calls=[]
            def fail_review(body):
                calls.append(body)
                if len(calls)==4:raise RuntimeError('fixture failure')
                return self.response()
            settings={'outputs_root':str(root/'outputs'),'native_llm':{'provider':'codex','model':'fixture'}}
            args=['--course',str(course),'--limit','all','--progress-file',str(progress),'--result-file',str(result)]
            with patch.object(self.s,'load_settings',return_value=settings), patch.object(self.s,'http_client',return_value=fail_review), patch.object(self.s.sys,'stdout',io.StringIO()):
                self.assertEqual(self.s.main(args),1)
            failed=json.loads(progress.read_text(encoding='utf-8'))
            self.assertEqual((failed['completed'],failed['current'],failed['stage']),(1,2,'failed'))
            calls.clear()
            def retry(body):calls.append(body);return self.response()
            with patch.object(self.s,'load_settings',return_value=settings), patch.object(self.s,'http_client',return_value=retry), patch.object(self.s.sys,'stdout',io.StringIO()):
                self.assertEqual(self.s.main(args),0)
            self.assertEqual(len(calls),1)
            self.assertEqual(json.loads(progress.read_text(encoding='utf-8'))['completed'],2)

    def test_native_bridge_uses_stdin_and_preserves_history_without_provider_options(self):
        settings={'native_llm':{'provider':'codex','model':'gpt-selected','api_key':'private'}}
        body={'messages':[{'role':'assistant','content':'draft'},{'role':'user','content':'review'}],**self.s.GENERATION}
        with patch.dict('os.environ',{'COURSE2MD_BIN':sys.executable}), patch.object(self.s.subprocess,'run') as run:
            run.return_value=type('Completed',(),{'returncode':0,'stdout':json.dumps(self.response()),'stderr':''})()
            response=self.s.http_client(settings)(body)
        args,kwargs=run.call_args
        self.assertEqual(args[0],[sys.executable,'ai-request'])
        payload=json.loads(kwargs['input'])
        self.assertEqual(payload['messages'],body['messages'])
        self.assertEqual(set(payload),{'settings','messages','max_tokens'})
        self.assertNotIn('private',str(args))
        self.assertEqual(self.s.parse_response(response)['title'],'例题')

    def test_native_identity_normalizes_endpoint_and_rejects_url_credentials(self):
        def identity(endpoint):return self.s.service_identity({'native_llm':{'provider':'openai-compatible','model':'selected','base_url':endpoint}})
        self.assertEqual(identity('https://EXAMPLE.com:443/v1/chat/completions/'),identity('https://example.com/v1'))
        for endpoint in ('https://user:password@example.com/v1','https://example.com/v1?api_key=private'):
            with self.assertRaises(ValueError):identity(endpoint)

    def test_native_stdin_rejects_invalid_json_and_unsupported_provider(self):
        for raw in ('not-json','[]','{"provider":"other","model":"x"}','{"provider":"codex","model":3}'):
            with self.assertRaises(ValueError):self.s.read_native_settings(io.StringIO(raw))
        self.assertEqual(self.s.read_native_settings(io.StringIO('{"provider":"codex","model":"selected","enabled":true}'))['model'],'selected')

    def test_native_missing_engine_cannot_fallback_to_old_qwen(self):
        with patch.dict('os.environ',{'COURSE2MD_BIN':''}):
            with self.assertRaises(ValueError):self.s.http_client({'native_llm':{'provider':'codex','model':'selected'}})

    def test_native_describe_consumes_selected_stdin_settings_once(self):
        with patch.object(self.s,'load_settings',return_value={'model':'stale','outputs_root':'out'}), patch.object(self.s.sys,'stdin',io.StringIO('{"provider":"codex","model":"selected","enabled":true}')), patch.object(self.s.sys,'stdout',io.StringIO()) as output:
            self.assertEqual(self.s.main(['--describe','--native-ai-stdin']),0)
            data=json.loads(output.getvalue())
        self.assertEqual((data['model'],data['provider'],data['auth_configured']),('selected','codex',True))

    def test_custom_prompt_envelope_describe_uses_exact_text_and_hash(self):
        prompt='用户完整指令\n<script>不要执行</script>\n$(New-Item marker)'
        envelope={'settings':{'provider':'codex','model':'selected','enabled':True},'prompt':prompt}
        with patch.object(self.s,'load_settings',return_value={'model':'stale'}), patch.object(self.s.sys,'stdin',io.StringIO(json.dumps(envelope))), patch.object(self.s.sys,'stdout',io.StringIO()) as output:
            self.assertEqual(self.s.main(['--describe','--native-ai-stdin']),0)
            data=json.loads(output.getvalue())
        self.assertEqual(data['prompt_source'],'custom')
        self.assertEqual(data['prompt_sha256'],self.s.hashlib.sha256(prompt.encode()).hexdigest())
        self.assertNotIn(prompt,output.getvalue())

    def test_custom_prompt_generation_changes_cache_and_preserves_verbatim_source(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);course=root/'course';course.mkdir();(course/'frame.png').write_bytes(b'image')
            self.s.write_json(course/'document.json',{'schema':1,'sections':[{'t':0,'end':1,'image':'frame.png','speech':[{'start':0,'end':1,'text':'原始材料'}]}]})
            settings={'outputs_root':str(root/'outputs')};received=[];paths=[]
            for prompt in ('完整自定义任务 A\n$(do not execute)','完整自定义任务 B'):
                result=root/'result.json'
                envelope={'settings':{'provider':'codex','model':'selected','enabled':True},'prompt':prompt}
                def client(body):
                    received.append(body['messages'][0]['content']);return self.response()
                with patch.object(self.s,'load_settings',return_value=dict(settings)), patch.object(self.s.sys,'stdin',io.StringIO(json.dumps(envelope))), patch.object(self.s,'http_client',return_value=client):
                    self.assertEqual(self.s.main(['--course',str(course),'--native-ai-stdin','--result-file',str(result)]),0)
                folder=Path(json.loads(result.read_text(encoding='utf-8'))['output_html']).parent;paths.append(folder)
                self.assertEqual((folder/'prompt-used.txt').read_text(encoding='utf-8'),prompt)
                metadata=json.loads((folder/'generation.json').read_text(encoding='utf-8'))
                self.assertEqual(metadata['prompt_sha256'],self.s.hashlib.sha256(prompt.encode()).hexdigest())
                self.assertEqual(metadata['prompt_sha256'],self.s.hashlib.sha256((folder/'prompt-used.txt').read_bytes()).hexdigest())
                self.assertEqual(metadata['prompt_source'],'custom')
            self.assertNotEqual(paths[0],paths[1])
            self.assertEqual(received,['完整自定义任务 A\n$(do not execute)']*2+['完整自定义任务 B']*2)

    def test_custom_prompt_export_requires_no_model_or_auth_and_records_identity(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);course=root/'course';course.mkdir();(course/'frame.png').write_bytes(b'image')
            self.s.write_json(course/'document.json',{'schema':1,'sections':[{'t':0,'end':1,'image':'frame.png','speech':[{'start':0,'end':1,'text':'原始材料'}]}]})
            prompt='手动上传完整自定义43指令';result=root/'result.json'
            envelope={'settings':{'model':'','provider':'open-ai-compatible'},'prompt':prompt}
            with patch.object(self.s,'load_settings',return_value={'outputs_root':str(root/'outputs')}), patch.object(self.s.sys,'stdin',io.StringIO(json.dumps(envelope))), patch.object(self.s,'http_client',side_effect=AssertionError('offline')):
                self.assertEqual(self.s.main(['--course',str(course),'--native-ai-stdin','--export-chatgpt','--result-file',str(result)]),0)
            data=json.loads(result.read_text(encoding='utf-8'));folder=Path(data['output_html']).parent
            with zipfile.ZipFile(data['export_zip']) as archive:
                self.assertEqual(archive.read('prompt43.txt').decode(),prompt)
                metadata=json.loads(archive.read('export.json'))
                self.assertEqual(metadata['prompt_sha256'],self.s.hashlib.sha256(archive.read('prompt-used.txt')).hexdigest())
            self.assertEqual(metadata['prompt_sha256'],self.s.hashlib.sha256(prompt.encode()).hexdigest())
            self.assertEqual((folder/'prompt-used.txt').read_text(encoding='utf-8'),prompt)

    def test_native_prompt_envelope_rejects_invalid_prompt_without_echoing_content(self):
        for prompt in ('   ',{},['private'],42):
            envelope={'settings':{'provider':'codex','model':'selected'},'prompt':prompt}
            with self.assertRaises(ValueError) as error:self.s.read_native_payload(io.StringIO(json.dumps(envelope)))
            self.assertNotIn('private',str(error.exception))
        for payload in ({'settings':[],'prompt':'x'},{'settings':{'model':'x'},'unexpected':'private'}):
            with self.assertRaises(ValueError):self.s.read_native_payload(io.StringIO(json.dumps(payload)))
        native,prompt=self.s.read_native_payload(io.StringIO('{"settings":{"provider":"codex","model":"selected"},"prompt":null}'))
        self.assertIsNone(prompt)
        self.assertEqual(native['model'],'selected')

    def test_manual_cli_export_honors_limit_without_auth_or_engine(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);course=root/'course';course.mkdir();(course/'frame.png').write_bytes(b'image')
            self.s.write_json(course/'document.json',{'schema':1,'sections':[
                {'t':i,'end':i+1,'image':'frame.png','speech':[{'start':i,'end':i+1,'text':'原始转录'}]} for i in range(4)]})
            result=root/'result.json'
            with patch.object(self.s,'load_settings',return_value={'outputs_root':str(root/'outputs')}), patch.object(self.s,'http_client',side_effect=AssertionError('offline')):
                self.assertEqual(self.s.main(['--course',str(course),'--limit','3','--export-chatgpt','--result-file',str(result)]),0)
            data=json.loads(result.read_text(encoding='utf-8'))
            self.assertEqual(len(data['selected_units']),3)
            self.assertEqual(data['mode'],'manual-chatgpt-export')
            self.assertTrue(Path(data['export_zip']).is_file())

    def test_manual_export_is_offline_complete_and_escapes_source(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td);image=p/'frame.png';image.write_bytes(b'image')
            unit={'id':'page-0001','course':'<script>alert(1)</script>','start':0,'end':4,'text':'<script>bad</script> transcript','images':[{'path':str(image),'t':0}]}
            with patch.object(self.s,'http_client',side_effect=AssertionError('must stay offline')):
                result=self.s.export_chatgpt(p/'out',[unit],'full43 prompt')
            self.assertEqual(result['status'],'success')
            self.assertIn('&lt;script&gt;',Path(result['output_html']).read_text(encoding='utf-8'))
            with zipfile.ZipFile(result['export_zip']) as archive:
                self.assertIn('prompt43.txt',archive.namelist())
                self.assertIn('transcript.txt',archive.namelist())
                self.assertTrue(any(n.startswith('images/') for n in archive.namelist()))
                self.assertEqual(archive.read('prompt43.txt').decode(),'full43 prompt')

    def setUp(self):
        path = Path(__file__).with_name('study.py')
        if not path.exists():
            self.fail('生成器尚未实现，必须先验证失败路径和来源保护')
        spec = importlib.util.spec_from_file_location('study', path)
        self.s = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.s)

    def response(self, reason='stop', text=None):
        if text is None:
            text = json.dumps({'title':'例题','overview':'学习目标', 'sections':[{'heading':'步骤','paragraphs':['先代入，再展开。']}], 'exercise':{'question':'计算','answer':'答案'}, 'uncertainties':[]})
        return {'choices':[{'finish_reason':reason,'message':{'content':text}}]}

    def test_truncation_and_empty_output_are_not_success(self):
        for obj in [self.response('length'), self.response(text=''), self.response(text='{"title":"假成功"}')]:
            with self.assertRaises(ValueError):
                self.s.parse_response(obj)
        self.assertEqual(self.s.parse_response(self.response())['title'],'例题')

    def test_prompt_image_model_changes_invalidate_cache(self):
        base = self.s.fingerprint('prompt', 'model', {'text':'a'}, [b'image'])
        for args in [('changed','model',{'text':'a'},[b'image']), ('prompt','other',{'text':'a'},[b'image']), ('prompt','model',{'text':'a'},[b'changed'])]:
            self.assertNotEqual(base,self.s.fingerprint(*args))

    def test_paths_follow_native_config_and_separate_ai_destination(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td);cfg=p/'config.toml';store=p/'storage.json';settings=p/'settings.json'
            cfg.write_text('[defaults]\nout = '+json.dumps(str(p/'native-selected'))+'\n',encoding='utf-8')
            self.s.write_json(store,{'ai_output':str(p/'ai-selected')})
            self.s.write_json(settings,{'config_path':str(cfg),'course_root':'stale-path','storage_settings':str(store)})
            actual=self.s.load_settings(settings)
            self.assertEqual(actual['course_root'],str(p/'native-selected'))
            self.assertEqual(actual['outputs_root'],str(p/'ai-selected'))

    def test_native_description_excludes_api_key_and_does_not_call_provider(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td);cfg=p/'config.toml'
            cfg.write_text('[llm]\nbase_url="https://example.com/v1"\napi_key="never-expose-this"\n',encoding='utf-8')
            settings={'config_path':str(cfg),'model':'real-model','outputs_root':str(p/'output')}
            data=self.s.describe(settings)
            self.assertEqual(data['model'],'real-model')
            self.assertTrue(data['api_key_present'])
            self.assertNotIn('never-expose-this',json.dumps(data))

    def test_native_job_receives_failure_manifest_on_early_failure(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/'job.json'
            with self.assertRaises((OSError,ValueError)):
                self.s.main(['--course',str(Path(td)/'missing'),'--result-file',str(path)])
            state=json.loads(path.read_text(encoding='utf-8'))
            self.assertEqual(state['status'],'failed')
            self.assertTrue(state['error'])

    def test_native_v2_document_without_timeline_keeps_image_and_speech_together(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td); (p/'frame.png').write_bytes(b'image')
            self.s.write_json(p/'document.json',{'schema':1,'sections':[
                {'t':0,'end':10,'image':'frame.png','speech':[{'start':1,'end':4,'text':'老师讲解'}]},
                {'t':10,'end':20,'image':'frame.png','speech':[]}]})
            units=self.s.load_course(p)
            self.assertEqual(len(units),1)
            self.assertEqual(units[0]['images'][0]['path'],str(p/'frame.png'))
            self.assertIn('老师讲解',units[0]['text'])

    def test_native_current_pointer_must_stay_inside_course(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)
            self.s.write_json(p/'current.json',{'schema':1,'manifest':'../outside/manifest.json'})
            with self.assertRaises(ValueError): self.s.load_course(p)

    def test_course_picker_lists_published_note_once_instead_of_each_version(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);course=root/'course';version=course/'versions/v1'
            version.mkdir(parents=True)
            (course/'timeline.jsonl').write_text('',encoding='utf-8')
            (version/'timeline.jsonl').write_text('',encoding='utf-8')
            self.s.write_json(course/'current.json',{'schema':1,'manifest':'versions/v1/manifest.json'})
            self.assertEqual(self.s.course_folders({'course_root':str(root)}),[course.resolve()])

    def test_known_equivalent_section_fields_are_normalized(self):
        o=json.loads(self.response()['choices'][0]['message']['content'])
        o['sections']=[{'title':'步骤','text':'先代入\n再展开'}]
        result=self.s.parse_response(self.response(text=json.dumps(o)))
        self.assertEqual(result['sections'],[{'heading':'步骤','paragraphs':['先代入\n再展开']}])

    def test_whole_json_fence_only(self):
        raw=self.response()['choices'][0]['message']['content']
        self.assertEqual(self.s.parse_response(self.response(text='```json\n'+raw+'\n```'))['title'],'例题')
        with self.assertRaises(ValueError):
            self.s.parse_response(self.response(text='额外猜测\n```json\n'+raw+'\n```'))

    def test_editorial_overlay_is_pinned_and_preserves_raw_result(self):
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td);original=self.s.parse_response(self.response())
            edit={'source_sha256':self.s.result_hash(original),'replacements':[{'old':'先代入，再展开。','new':'先说明条件，再代入。','reason':'补充前提'}]}
            self.s.write_json(folder/'u.editorial.json',edit)
            fixed,notes=self.s.apply_editorial(folder,'u',original)
            self.assertEqual(original['sections'][0]['paragraphs'][0],'先代入，再展开。')
            self.assertEqual(fixed['sections'][0]['paragraphs'][0],'先说明条件，再代入。')
            self.assertEqual(notes,['补充前提'])
            original['title']='不同回复'
            with self.assertRaises(ValueError):self.s.apply_editorial(folder,'u',original)

    def test_blank_spacers_are_removed_but_empty_sections_fail(self):
        obj=json.loads(self.response()['choices'][0]['message']['content'])
        obj['sections'][0]['paragraphs']=['第一步','','  ','第二步']
        result=self.s.parse_response(self.response(text=json.dumps(obj)))
        self.assertEqual(result['sections'][0]['paragraphs'],['第一步','第二步'])
        obj['sections'][0]['paragraphs']=['','  ']
        with self.assertRaises(ValueError):self.s.parse_response(self.response(text=json.dumps(obj)))

    def test_failed_request_is_recorded_without_fake_lesson(self):
        with tempfile.TemporaryDirectory() as td:
            dest=Path(td)
            def fail(_): raise RuntimeError('HTTP 404 model_not_found')
            with self.assertRaises(RuntimeError):
                self.s.run_request(dest, 'u1', 'hash', {}, fail)
            status=json.loads((dest/'u1.status.json').read_text())
            self.assertEqual(status['status'],'failed')
            self.assertIn('404',status['error'])
            self.assertFalse((dest/'u1.json').exists())

    def test_same_success_resumes_without_second_api_call(self):
        with tempfile.TemporaryDirectory() as td:
            calls=[]
            def ok(_): calls.append(1); return self.response()
            for _ in range(2): self.s.run_request(Path(td),'u1','hash',{},ok)
            self.assertEqual(len(calls),1)

    def test_review_failure_does_not_publish_draft_as_final(self):
        with tempfile.TemporaryDirectory() as td:
            calls=[]
            def api(_):
                calls.append(1)
                if len(calls)==2: raise RuntimeError('review failed')
                return self.response()
            if not hasattr(self.s,'generate_with_review'):
                self.fail('缺少生成后校核关卡')
            with self.assertRaises(RuntimeError):
                self.s.generate_with_review(Path(td),'u','hash',{'messages':[]},api)
            self.assertEqual(len(calls),2)
            status=json.loads((Path(td)/'u.review.status.json').read_text())
            self.assertEqual(status['status'],'failed')

    def test_failed_review_clears_previous_final_success(self):
        with tempfile.TemporaryDirectory() as td:
            folder=Path(td)
            self.s.generate_with_review(folder,'u','old',{'messages':[]},lambda _:self.response())
            calls=[]
            def fail_review(_):
                calls.append(1)
                if len(calls)==2: raise RuntimeError('new review failed')
                return self.response()
            with self.assertRaises(RuntimeError):
                self.s.generate_with_review(folder,'u','new',{'messages':[]},fail_review)
            status=json.loads((folder/'u.final.status.json').read_text(encoding='utf-8'))
            self.assertEqual(status['status'],'failed')

    def test_renderer_escapes_model_html(self):
        with tempfile.TemporaryDirectory() as td:
            result=self.s.parse_response(self.response())
            result['title']='<script>alert(1)</script>'
            unit={'id':'u','course':'c','start':0,'end':1,'images':[],'text':'<iframe>'}
            self.s.render(Path(td),[(unit,result,{})],'model')
            output=(Path(td)/'index.html').read_text(encoding='utf-8')
            self.assertNotIn('<script>',output)
            self.assertIn('&lt;script&gt;',output)

    def test_renderer_typesets_math_in_body_and_answer(self):
        with tempfile.TemporaryDirectory() as td:
            result=self.s.parse_response(self.response())
            result['sections'][0]['paragraphs']=[r'共轭 $\overline{z}$；分数 $$\frac{1}{z}$$']
            result['exercise']['answer']=r'答案 $x_1^2$'
            unit={'id':'u','course':'c','start':0,'end':1,'images':[],'text':'原文'}
            self.s.render(Path(td),[(unit,result,{})],'model')
            output=(Path(td)/'index.html').read_text(encoding='utf-8')
            self.assertIn('<mfrac>',output)
            self.assertIn('<mover',output)
            self.assertIn('<msubsup',output)
            report=json.loads((Path(td)/'math-report.json').read_text(encoding='utf-8'))
            self.assertEqual(report['errors'],[])

    def test_course_loader_keeps_boundary_speech_and_rejects_escape(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td);(p/'frames').mkdir();(p/'frames/a.jpg').write_bytes(b'a');(p/'frames/b.jpg').write_bytes(b'b')
            events=[{'type':'frame','t':0,'image':'frames/a.jpg'}, {'type':'speech','start':9,'end':11,'text':'跨页完整一句'}, {'type':'frame','t':10,'image':'frames/b.jpg'}]
            raw='\n'.join(json.dumps(x,ensure_ascii=False) for x in events)
            (p/'timeline.jsonl').write_text(raw,encoding='utf-8')
            units=self.s.load_course(p)
            self.assertTrue(all('跨页完整一句' in u['text'] for u in units))
            self.assertEqual((p/'timeline.jsonl').read_text(encoding='utf-8'),raw)
            events[0]['image']='../escape.jpg'
            (p/'timeline.jsonl').write_text('\n'.join(json.dumps(x) for x in events))
            with self.assertRaises(ValueError): self.s.load_course(p)


if __name__=='__main__': unittest.main()

