import importlib.util
from pathlib import Path
import tempfile
import unittest


class MathTests(unittest.TestCase):
    def setUp(self):
        path=Path(__file__).with_name('math_render.py')
        self.assertTrue(path.exists(),'缺少数学排版模块，当前公式仍为纯文字')
        spec=importlib.util.spec_from_file_location('math_render',path)
        self.m=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.m)

    def test_real_math_structures_and_local_fonts(self):
        formulas=[
            (r'$$\frac{a+b}{c+d}$$','<mfrac>'),
            (r'$$\int_0^1 x^2\,dx = \frac{1}{3}$$','∫'),
            (r'$$\overline{z}_1=x_1-iy_1$$','<mover'),
            (r'$$\sum_{n=1}^{\infty}\frac{1}{n^2}$$','∑'),
            (r'$$\lim_{x\to0}\frac{\sin x}{x}=1$$','<munder'),
            (r'$$\begin{pmatrix}1&2\\3&4\end{pmatrix}$$','<mtable'),
            (r'$$f(x)=\begin{cases}x^2&x\ge0\\-x&x<0\end{cases}$$','<mtable'),
            (r'$$\sqrt{x^2+y^2}\le\alpha+\beta$$','<msqrt>'),
            (r'$$\frac{\partial f}{\partial x}\ne0$$','∂'),
        ]
        with tempfile.TemporaryDirectory() as td:
            renderer=self.m.MathRenderer(Path(td),[x for x,_ in formulas])
            for source,node in formulas:
                self.assertIn(node,renderer.text(source))
            self.assertEqual(renderer.errors,[])
            self.assertTrue((Path(td)/'math/katex.min.css').is_file())
            self.assertTrue(list((Path(td)/'math/fonts').glob('*.woff2')))

    def test_inline_math_and_text_are_escaped(self):
        source=r'先看 $z_1=x_1+iy_1$，再看 <script>alert(1)</script>。'
        with tempfile.TemporaryDirectory() as td:
            renderer=self.m.MathRenderer(Path(td),[source])
            out=renderer.text(source)
            self.assertIn('class="katex"',out)
            self.assertNotIn('<script>',out)
            self.assertIn('&lt;script&gt;',out)

    def test_bad_math_is_visible_and_source_preserved(self):
        sources=[r'坏公式 $\unknownmath{x}$',r'缺右边 $$\frac{x}{y}', '转义损坏 $\x0crac{x}{y}$',r'漏了定界符 \frac{x}{y}']
        with tempfile.TemporaryDirectory() as td:
            renderer=self.m.MathRenderer(Path(td),sources)
            for s in sources:self.assertIn('公式排版待修复',renderer.text(s))
            self.assertEqual(len(renderer.errors),4)
            self.assertIn('[U+000C]',renderer.markdown(sources[2]))
            self.assertNotIn('\x0c',renderer.markdown(sources[2]))

    def test_links_inside_math_cannot_create_html(self):
        source=r'$\href{https://example.com}{x}$'
        with tempfile.TemporaryDirectory() as td:
            renderer=self.m.MathRenderer(Path(td),[source])
            self.assertIn('公式排版待修复',renderer.text(source))
            self.assertNotIn('<a ',renderer.text(source))

    def test_legacy_slash_is_not_guessed_into_a_fraction(self):
        source='w = 1/2z'
        with tempfile.TemporaryDirectory() as td:
            renderer=self.m.MathRenderer(Path(td),[source])
            self.assertNotIn('<mfrac>',renderer.text(source))
            self.assertIn('1/2z',renderer.markdown(source))

    def test_legacy_conjugate_and_subscripts_keep_meaning(self):
        source='共轭 z̄₁ = x₁ - iy₁。性质 i² = -1。'
        with tempfile.TemporaryDirectory() as td:
            renderer=self.m.MathRenderer(Path(td),[source])
            out=renderer.text(source)
            self.assertIn('<mover',out)
            self.assertIn('<msub',out)
            self.assertIn('<msup',out)
            self.assertEqual(renderer.errors,[])


if __name__=='__main__':unittest.main()
