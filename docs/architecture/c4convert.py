#!/usr/bin/env python3
"""
C4 DSL 批量转换工具
用法: python3 C4convert.py --dir ../assets
"""

import os
import sys
import argparse
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from datetime import datetime

class C4Converter:
    def __init__(self, dsl_dir, suffix=None, png=True, mermaid=False, force=False):
        """
        初始化转换器
        :param dsl_dir: DSL 文件所在目录
        :param suffix: 文件后缀 (默认自动检测)
        """
        self.dsl_dir = Path(dsl_dir)
        self.suffix = suffix
        self.png = png
        self.mermaid = mermaid
        self.force = force
        self.structurizr_cmd = "structurizr.sh"  # 或者 "structurizr"
        self.results = []
        
    def find_dsl_files(self):
        """查找所有 DSL 文件"""
        dsl_files = []
        
        if self.suffix:
            # 指定了后缀
            pattern = f"*.{self.suffix}"
            dsl_files = list(self.dsl_dir.glob(pattern))
        else:
            # 自动检测常见后缀
            dsl_files = []
            non_puml_stems = set()
            for ext in ['dsl', 'workspace', 'c4']:
                for f in self.dsl_dir.glob(f"*.{ext}"):
                    dsl_files.append(f)
                    non_puml_stems.add(f.stem)

            for f in self.dsl_dir.glob("*.puml"):
                if f.stem not in non_puml_stems:
                    dsl_files.append(f)
        
        if not dsl_files:
            print(f"⚠️  未找到任何 .dsl/.workspace/.c4/.puml 文件在 {self.dsl_dir}")
            return []
        
        print(f"📁 找到 {len(dsl_files)} 个文件:")
        for f in dsl_files:
            print(f"   - {f.name}")
        return dsl_files
    
    def convert_file(self, dsl_path):
        """
        转换单个 DSL 文件
        :param dsl_path: DSL 文件路径
        :return: 转换结果
        """
        if dsl_path.suffix.lower() == ".puml" or self._is_plantuml_source(dsl_path):
            return self.convert_plantuml_file(dsl_path)

        base_name = dsl_path.stem
        output_dir = dsl_path.parent

        result = {
            "dsl_file": str(dsl_path),
            "base_name": base_name,
            "output_dir": str(output_dir),
            "success": False,
            "errors": []
        }

        if not self.force and self._outputs_exist(dsl_path):
            result["success"] = True
            result["skipped"] = True
            print(f"\n⏭️  跳过: {dsl_path.name}（所需文件已存在）")
            return result

        print(f"\n🔄 处理: {dsl_path.name}")
        print(f"   📂 输出目录: {output_dir}")

        try:
            with tempfile.TemporaryDirectory(prefix=f".{base_name}-", dir=output_dir) as tmp:
                tmp_dir = Path(tmp)

                if self.mermaid:
                    print(f"   📝 生成 Mermaid...")
                    mermaid_cmd = [
                        self.structurizr_cmd,
                        "export",
                        "-workspace", str(dsl_path),
                        "-format", "mermaid",
                        "-output", str(tmp_dir)
                    ]

                    mermaid_result = subprocess.run(
                        mermaid_cmd,
                        capture_output=True,
                        text=True,
                        check=False
                    )

                    if mermaid_result.returncode != 0:
                        error_msg = f"Mermaid 生成失败: {mermaid_result.stderr}"
                        result["errors"].append(error_msg)
                        print(f"   ❌ {error_msg}")
                    else:
                        generated_files = list(tmp_dir.glob("*.mermaid"))
                        if not generated_files:
                            generated_files = list(tmp_dir.glob("*.mmd"))
                        if generated_files:
                            src = generated_files[0]
                            mermaid_file = output_dir / f"{base_name}.mermaid.md"

                            content = src.read_text()
                            if not content.startswith("```mermaid"):
                                content = f"```mermaid\n{content}\n```"
                            mermaid_file.write_text(content)
                            src.unlink()

                            result["mermaid_file"] = str(mermaid_file)
                            print(f"   ✅ Mermaid: {mermaid_file.name}")

                print(f"   🖼️  生成 PlantUML...")
                plantuml_cmd = [
                    self.structurizr_cmd,
                    "export",
                    "-workspace", str(dsl_path),
                    "-format", "plantuml/c4plantuml",
                    "-output", str(tmp_dir)
                ]

                plantuml_result = subprocess.run(
                    plantuml_cmd,
                    capture_output=True,
                    text=True,
                    check=False
                )

                if plantuml_result.returncode != 0:
                    error_msg = f"PlantUML 生成失败: {plantuml_result.stderr}"
                    result["errors"].append(error_msg)
                    print(f"   ❌ {error_msg}")
                else:
                    generated_files = list(tmp_dir.glob("*.puml"))
                    if generated_files:
                        src = generated_files[0]
                        puml_file = output_dir / f"{base_name}.puml"
                        shutil.move(str(src), str(puml_file))
                        self._make_plantuml_compatible(puml_file)
                        result["plantuml_file"] = str(puml_file)
                        print(f"   ✅ PlantUML: {puml_file.name}")

                if self.png:
                    if self.check_plantuml_installed():
                        print(f"   🎨 渲染 PNG...")
                        png_files = self.render_images(output_dir, base_name)
                        if png_files:
                            result["image_files"] = png_files
                            for png in png_files:
                                print(f"      ✅ {Path(png).name}")
                    else:
                        print(f"   ⚠️  plantuml 未安装，跳过 PNG 渲染")
                        print(f"   💡 安装: sudo apt install plantuml")
            
            if not result["errors"]:
                result["success"] = True
                print(f"   ✅ 完成")
            else:
                print(f"   ⚠️  部分完成，但有错误")
                
        except Exception as e:
            error_msg = f"转换异常: {str(e)}"
            result["errors"].append(error_msg)
            print(f"   ❌ {error_msg}")
        
        return result

    def _is_plantuml_source(self, source_path):
        """判断文件内容是否为 PlantUML 源码。"""
        try:
            with source_path.open(encoding="utf-8") as f:
                for line in f:
                    stripped = line.strip()
                    if not stripped or stripped.startswith("'"):
                        continue
                    return stripped.startswith("@startuml")
        except OSError:
            return False

        return False

    def convert_plantuml_file(self, source_path):
        """直接转换 PlantUML 源文件，不经过 Structurizr CLI。"""
        base_name = source_path.stem
        output_dir = source_path.parent

        result = {
            "dsl_file": str(source_path),
            "base_name": base_name,
            "output_dir": str(output_dir),
            "success": False,
            "errors": []
        }

        if not self.force and self._outputs_exist(source_path):
            result["success"] = True
            result["skipped"] = True
            print(f"\n⏭️  跳过: {source_path.name}（所需文件已存在）")
            return result

        print(f"\n🔄 处理: {source_path.name} (PlantUML)")
        print(f"   📂 输出目录: {output_dir}")

        try:
            result["source_file"] = str(source_path)
            print(f"   📄 输入源: {source_path.name}")

            result["mermaid_skipped"] = True

            if self.png:
                if self.check_plantuml_installed():
                    print(f"   🎨 渲染 PNG...")
                    png_files = self.render_images(output_dir, base_name)
                    if png_files:
                        result["image_files"] = png_files
                        for png in png_files:
                            print(f"      ✅ {Path(png).name}")
                else:
                    print(f"   ⚠️  plantuml 未安装，跳过 PNG 渲染")
                    print(f"   💡 安装: sudo apt install plantuml")

            if not result["errors"]:
                result["success"] = True
                print(f"   ✅ 完成")
            else:
                print(f"   ⚠️  部分完成，但有错误")

        except Exception as e:
            error_msg = f"转换异常: {str(e)}"
            result["errors"].append(error_msg)
            print(f"   ❌ {error_msg}")

        return result
    
    def check_plantuml_installed(self):
        """检查 plantuml 是否安装"""
        try:
            result = subprocess.run(
                ["plantuml", "-version"],
                capture_output=True,
                text=True,
                check=False
            )
            return result.returncode == 0
        except FileNotFoundError:
            return False

    def _make_plantuml_compatible(self, puml_file):
        """调整 Structurizr 生成的 PlantUML，兼容当前 PlantUML/C4 标准库。"""
        path = Path(puml_file)
        lines = path.read_text().splitlines()
        has_default_font = any("skinparam defaultFontName" in line for line in lines)
        cleaned_lines = []

        for line in lines:
            if line.strip().startswith("SHOW_LEGEND"):
                continue

            # Structurizr 会输出空的可选参数，旧版 PlantUML/C4 宏不支持。
            line = re.sub(r',\s*\$[A-Za-z_]+=""(?=[,)])', "", line)

            if line.strip() == "@startuml":
                cleaned_lines.append(line)
                if not has_default_font:
                    cleaned_lines.append('skinparam defaultFontName "WenQuanYi Micro Hei"')
                continue

            cleaned_lines.append(line)

        path.write_text("\n".join(cleaned_lines) + "\n")
    
    def render_images(self, output_dir, base_name):
        """渲染 PNG 图片"""
        puml_file = output_dir / f"{base_name}.puml"
        png_file = output_dir / f"{base_name}.png"
        
        if not puml_file.exists():
            return []
        
        try:
            # 尝试直接使用 plantuml 命令
            result = subprocess.run(
                ["plantuml", "-tpng", puml_file.name],
                cwd=output_dir,
                capture_output=True,
                text=True,
                check=False
            )
            
            if result.returncode == 0:
                if png_file.exists():
                    return [str(png_file)]
            else:
                print(f"      ⚠️  渲染失败: {result.stderr}")
        except Exception as e:
            print(f"      ⚠️  渲染异常: {e}")
        
        return []

    def _expected_outputs(self, source_path):
        """返回该源文件在默认同目录输出模式下应生成的产物。"""
        base_name = source_path.stem
        output_dir = source_path.parent
        outputs = []

        is_plantuml_source = (
            source_path.suffix.lower() == ".puml"
            or self._is_plantuml_source(source_path)
        )

        if not is_plantuml_source:
            outputs.append(output_dir / f"{base_name}.puml")
            if self.mermaid:
                outputs.append(output_dir / f"{base_name}.mermaid.md")

        if self.png:
            outputs.append(output_dir / f"{base_name}.png")

        return outputs

    def _outputs_exist(self, source_path):
        """检查懒构建条件：所需输出文件是否已全部存在且不早于源文件。"""
        try:
            src_mtime = source_path.stat().st_mtime_ns
        except OSError:
            return False

        for output in self._expected_outputs(source_path):
            if not output.exists():
                return False
            try:
                if output.stat().st_mtime_ns < src_mtime:
                    return False
            except OSError:
                return False

        return True
    
    def convert_all(self):
        """转换所有 DSL 文件"""
        dsl_files = self.find_dsl_files()
        if not dsl_files:
            return False
        
        print("\n" + "="*60)
        print("🚀 开始批量转换...")
        print("="*60)
        
        for dsl_file in dsl_files:
            result = self.convert_file(dsl_file)
            self.results.append(result)
        
        # 打印汇总
        self.print_summary()
        if any(not r.get("skipped", False) for r in self.results) or not (self.dsl_dir / "README.md").exists():
            self.generate_index()
        else:
            print("\n📄 索引已是最新，跳过生成")
        return all(r["success"] for r in self.results)
    
    def print_summary(self):
        """打印汇总信息"""
        print("\n" + "="*60)
        print("📊 转换汇总")
        print("="*60)
        
        success_count = sum(1 for r in self.results if r["success"])
        total_count = len(self.results)
        
        print(f"✅ 成功: {success_count}/{total_count}\n")
        
        for r in self.results:
            status = "✅" if r["success"] else "❌"
            print(f"   {status} {r['base_name']}")
            if "mermaid_file" in r:
                print(f"      📝 {Path(r['mermaid_file']).name}")
            if r.get("mermaid_skipped"):
                print(f"      ⏭️  Mermaid: 不适用（PlantUML 输入）")
            if r.get("skipped"):
                print(f"      ⏭️  已是最新，跳过生成")
            if "plantuml_file" in r:
                print(f"      🖼️  {Path(r['plantuml_file']).name}")
            if "image_files" in r and r["image_files"]:
                print(f"      🖼️  {len(r['image_files'])} 张图片")
            if r["errors"]:
                for err in r["errors"]:
                    print(f"      ⚠️  {err}")
    
    def generate_index(self):
        """生成索引文件"""
        index_file = self.dsl_dir / "README.md"
        content = []
        content.append("# C4 DSL 图表\n")
        content.append(f"生成时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n")
        content.append("## 图表列表\n")
        
        for r in self.results:
            if r["success"]:
                content.append(f"### {r['base_name']}\n")
                
                if "mermaid_file" in r:
                    mermaid_name = Path(r['mermaid_file']).name
                    content.append(f"- Mermaid: [`{mermaid_name}`]({mermaid_name})\n")
                
            if "image_files" in r and r["image_files"]:
                for img in r["image_files"]:
                    img_name = Path(img).name
                    content.append(f"- 图片: ![{img_name}]({img_name})\n")
                if "source_file" in r:
                    source_name = Path(r["source_file"]).name
                    content.append(f"- PlantUML: [`{source_name}`]({source_name})\n")
            elif "plantuml_file" in r:
                puml_name = Path(r['plantuml_file']).name
                content.append(f"- PlantUML: [`{puml_name}`]({puml_name})\n")
                
                content.append("\n")
        
        index_file.write_text("\n".join(content))
        print(f"\n📄 索引: {index_file}")

def main():
    parser = argparse.ArgumentParser(
        description="C4 DSL 批量转换工具",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  # 处理目录下所有 .dsl 文件
  python3 convert.py --dir ../assets
  
  # 处理 .workspace 文件
  python3 convert.py --dir ../assets --suffix workspace
  
  # 处理当前目录
  python3 convert.py --dir .
  
  # 默认输出到源文件同目录，并生成 PNG；不生成 Mermaid
  python3 convert.py --dir ../assets
  
  # 关闭 PNG
  python3 convert.py --dir ../assets --no-png
  
  # 生成 Mermaid
  python3 convert.py --dir ../assets --mermaid
  
  # 强制重新生成
  python3 convert.py --dir ../assets -f
        """
    )
    
    parser.add_argument(
        "--dir",
        required=True,
        help="包含 DSL 文件的目录路径"
    )
    
    parser.add_argument(
        "--suffix", "-s",
        help="文件后缀 (默认自动检测: dsl, workspace, c4)"
    )

    parser.add_argument(
        "--png",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="是否渲染 PNG（默认开启，使用 --no-png 关闭）"
    )

    parser.add_argument(
        "--mermaid",
        action="store_true",
        default=False,
        help="是否生成 Mermaid（默认关闭）"
    )

    parser.add_argument(
        "--memraid",
        dest="mermaid",
        action="store_true",
        help=argparse.SUPPRESS
    )

    parser.add_argument(
        "-f", "--force",
        action="store_true",
        default=False,
        help="强制重新生成所有文件"
    )
    
    args = parser.parse_args()
    
    if not os.path.exists(args.dir):
        print(f"❌ 错误: 目录不存在 '{args.dir}'")
        sys.exit(1)
    
    converter = C4Converter(
        args.dir,
        args.suffix,
        png=args.png,
        mermaid=args.mermaid,
        force=args.force
    )
    success = converter.convert_all()
    sys.exit(0 if success else 1)

if __name__ == "__main__":
    main()
