#!/usr/bin/env python3
"""检查 Markdown 本地链接和 code-layout 中已完成的文件路径。"""

import argparse
import os
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit

from repository import Repository


class DocumentationCheck:
    link = re.compile(r'!?\[[^\]\n]*\]\(\s*')
    reference = re.compile(r'^\s{0,3}\[[^\]]+\]:\s*(<[^>]+>|\S+)')
    node = re.compile(r'^(?P<prefix>[│ ]*)(?:├──|└──)\s+(?P<body>.*)$')
    state = re.compile(r'^\[([ x/\-])\]\s+(.*)$')

    def __init__(self, min_layout_files):
        self.repo = Repository()
        self.findings = {}
        self.min_layout_files = min_layout_files
        self.layout_files = set()

    @classmethod
    def from_arguments(cls):
        parser = argparse.ArgumentParser(description='Check documentation links and code layouts.')
        parser.add_argument(
            '--min-layout-files', type=int,
            default=os.environ.get('ACTRAIL_CI_MIN_LAYOUT_FILES', '10'),
            help='minimum existing files listed in a separate code layout '
                 '(default: ACTRAIL_CI_MIN_LAYOUT_FILES, or 10 when unset; current: %(default)s)',
        )
        args = parser.parse_args()
        if args.min_layout_files < 1:
            parser.error('--min-layout-files must be positive')
        return cls(args.min_layout_files)

    def report(self, line, message, target=None):
        self.findings.setdefault(message, []).append((line, target))

    def report_document(self, path):
        if not self.findings:
            return
        lines = sorted({line for items in self.findings.values() for line, _ in items})
        self.repo.report(path, lines, 'Documentation issues')
        for message, items in self.findings.items():
            group_lines = sorted({line for line, _ in items})
            suffix = f': lines {group_lines}' if len(self.findings) > 1 else ''
            print(f'  {message}{suffix}')
            for line, target in sorted(set(items)):
                if target is not None:
                    print(f'    {line}: `{target}`')

    def targets(self, prose):
        # 配对目标中的括号，避免把合法文件名中的括号当作链接结尾。
        for match in self.link.finditer(prose):
            start = match.end()
            if prose[start:start + 1] == '<':
                end = prose.find('>', start + 1)
                if end >= 0:
                    yield prose[start + 1:end]
                continue
            cursor = start
            depth = 0
            while cursor < len(prose):
                char = prose[cursor]
                if char == '\\':
                    cursor += 2
                    continue
                if char == '(':
                    depth += 1
                elif char == ')':
                    if depth == 0:
                        yield re.sub(r'\\([()\\])', r'\1', prose[start:cursor])
                        break
                    depth -= 1
                elif char.isspace() and depth == 0:
                    if re.match(r'\s+(?:"[^"]*"|\x27[^\x27]*\x27|\([^)]*\))?\s*\)', prose[cursor:]):
                        yield prose[start:cursor]
                    break
                cursor += 1

    def check_link(self, document, line, target):
        target = target.strip('<>')
        parsed = urlsplit(target)
        if parsed.scheme or parsed.netloc or not parsed.path:
            return
        target = unquote(parsed.path)
        if not ((document.parent / target).exists() or (self.repo.root / target).exists()):
            self.report(line, 'Local link target does not exist', target)

    def check_tree(self, lines):
        # 树根相对仓库。文件节点下的 struct、方法等定义不属于文件路径。
        stack = []
        for number, text in lines:
            branch = self.node.match(text)
            if branch:
                prefix = branch['prefix']
                if len(prefix) % 4:
                    self.report(number, 'Layout tree indentation must use four columns per level')
                    continue
                depth = len(prefix) // 4 + 1
                body = branch['body']
            else:
                if not text.strip():
                    continue
                depth = 0
                body = text.strip()
            state = self.state.match(body)
            status, name = (state[1], state[2]) if state else ('', body)
            name = name.split(' #', 1)[0].strip()
            if depth == 0:
                if not (name == './' or (state and name.endswith('/'))):
                    self.report(number, 'Unrecognized layout tree root', body)
                    stack = []
                    continue
                path = self.repo.root / name
            else:
                if depth > len(stack):
                    self.report(number, 'Layout tree node has no parent')
                    continue
                parent = stack[depth - 1]
                path = parent / name if parent is not None else None
            directory = name.endswith('/')
            stack[depth:] = [path if directory else None]
            if path is None:
                continue
            resolved = path.resolve()
            if not resolved.is_relative_to(self.repo.root):
                self.report(number, 'Layout path is outside the repository', name)
                continue
            valid = path.is_dir() if directory else path.is_file()
            if valid and not directory:
                self.layout_files.add(resolved)
            if status == 'x':
                if not valid:
                    self.report(number, 'Completed layout path is missing or has the wrong type', str(path.relative_to(self.repo.root)))
                    # 父目录失效时，其子路径不再重复报告同一原因。
                    if directory:
                        stack[depth] = None

    def check_document(self, path):
        self.findings = {}
        self.layout_files = set()
        fence = None
        tree = []
        layout = path.name == 'code-layout.md'
        for number, line in enumerate(path.read_text(encoding='utf-8').splitlines(), 1):
            marker = re.match(r'^\s{0,3}(`{3,}|~{3,})(.*)$', line)
            if marker:
                if fence is None:
                    fence = marker[1]
                    tree = [] if layout and marker[2].strip() in ('text', '') else None
                elif marker[1][0] == fence[0] and len(marker[1]) >= len(fence) and not marker[2].strip():
                    if tree:
                        self.check_tree(tree)
                    fence = None
                continue
            if fence is not None:
                if tree is not None:
                    tree.append((number, line))
                continue
            # 代码示例中的链接不参与文档路径检查。
            prose = re.sub(r'(`+).*?\1', '', line)
            for target in self.targets(prose):
                self.check_link(path, number, target)
            reference = self.reference.match(prose)
            if reference:
                self.check_link(path, number, reference[1])
        if fence is not None:
            self.report(number, 'Markdown code fence is not closed')
        if layout and path != self.repo.root / 'docs/architecture/code-layout.md':
            if len(self.layout_files) < self.min_layout_files:
                self.report(1, f'Layout lists {len(self.layout_files)} files; minimum is '
                            f'{self.min_layout_files}. Merge it into the parent layout')

    def run(self):
        count = 0
        for path in self.repo.files('.md'):
            count += 1
            self.check_document(path)
            self.report_document(path)
        return self.repo.finish('documentation', count)


if __name__ == '__main__':
    try:
        sys.exit(DocumentationCheck.from_arguments().run())
    except (OSError, UnicodeError, ValueError, subprocess.CalledProcessError) as error:
        print(f'ERROR: {error}', file=sys.stderr)
        sys.exit(2)
