#!/usr/bin/env python3
"""检查 Rust 主代码中的测试属性，要求测试放在独立 tests 目录。"""

import re
import subprocess
import sys

from repository import Repository


class TestHygieneCheck:
    # 遮蔽注释、字符串和字符字面量，保留换行以定位诊断。
    literal = re.compile(
        r'//[^\n]*|/\*|r(?P<hash>\#*)".*?"(?P=hash)|'
        r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\\n])\'', re.S,
    )
    attribute = re.compile(
        r'#\s*\[\s*(?:(?:tokio|async_std)\s*::\s*)?test\s*(?:\([^\]]*\))?\s*\]'
    )

    def __init__(self):
        self.repo = Repository()

    def source(self, text):
        # Rust 块注释允许嵌套，逐层跳过后继续扫描。
        parts = []
        offset = 0
        while match := self.literal.search(text, offset):
            parts.append(text[offset:match.start()])
            end = match.end()
            if match.group() == "/*":
                depth = 1
                while depth and end < len(text):
                    if text.startswith("/*", end):
                        depth += 1
                        end += 2
                    elif text.startswith("*/", end):
                        depth -= 1
                        end += 2
                    else:
                        end += 1
            parts.append(re.sub(r'[^\n]', ' ', text[match.start():end]))
            offset = end
        parts.append(text[offset:])
        return ''.join(parts)

    def run(self):
        count = 0
        for path in self.repo.files('.rs'):
            parts = path.relative_to(self.repo.root).parts
            # 独立 tests 下的 fixture 可以拥有 src，src/tests 仍属于主代码。
            if 'tests' in parts and ('src' not in parts or parts.index('tests') < parts.index('src')):
                continue
            count += 1
            source = self.source(path.read_text(encoding='utf-8'))
            lines = sorted({
                source.count('\n', 0, match.start()) + 1
                for match in self.attribute.finditer(source)
            })
            if lines:
                self.repo.report(
                    path, lines,
                    'Test attributes in production code. Move tests to a `tests/` directory outside `src/`.',
                )
        return self.repo.finish('test hygiene', count)


if __name__ == '__main__':
    try:
        sys.exit(TestHygieneCheck().run())
    except (OSError, UnicodeError, subprocess.CalledProcessError) as error:
        print(f'ERROR: {error}', file=sys.stderr)
        sys.exit(2)
