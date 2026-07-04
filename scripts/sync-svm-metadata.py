#!/usr/bin/env python3
"""Compare SVM C# models against Quartermaster metadata and generate stubs for missing fields.

Usage:
    ./scripts/sync-svm-metadata.py <path-to-svm-repo>

Parses the SVM C# model files, extracts every field path and its type,
then diffs against the paths already in src/svm/metadata.rs.

For each missing field, prints a FieldMeta Rust stub with:
  - InputType derived from C# type (bool→Toggle, int→Integer, double→Float, string→Text)
  - Label derived from the field name
  - Placeholder description
  - Subgroup from the parent struct name

Review and edit the output before pasting into metadata.rs.
"""

import os
import re
import sys


def parse_models(models_dir):
    """Parse C# model files and return {class_name: [(field_name, field_type)]}."""
    types = {}
    for root, _, files in os.walk(models_dir):
        for f in files:
            if not f.endswith(".cs"):
                continue
            path = os.path.join(root, f)
            content = open(path).read()
            lines = content.split("\n")

            for cls in re.findall(r"public class (\w+)", content):
                if cls not in types:
                    types[cls] = []

            for line in lines:
                stripped = line.strip()
                if stripped.startswith("//"):
                    continue
                m = re.match(r"public\s+(\w+)\s+(\w+)\s*\{", stripped)
                if not m:
                    continue
                ftype, fname = m.group(1), m.group(2)
                pos = content.find(stripped)
                class_before = None
                for cm in re.finditer(r"public class (\w+)", content[:pos]):
                    class_before = cm.group(1)
                if class_before and class_before in types:
                    types[class_before].append((fname, ftype))
    return types


PRIMITIVE_TYPES = {"int", "double", "bool", "string", "String"}


def flatten(types, cls_name, prefix=""):
    """Recursively flatten a class into (path, csharp_type) tuples."""
    if cls_name not in types:
        return []
    result = []
    for fname, ftype in types[cls_name]:
        path = f"{prefix}{fname}" if prefix else fname
        if ftype in PRIMITIVE_TYPES:
            result.append((path, ftype))
        else:
            result.extend(flatten(types, ftype, f"{path}."))
    return result


def get_svm_fields(svm_repo):
    """Get all SVM fields as {path: csharp_type}."""
    models_dir = os.path.join(svm_repo, "Models", "Models")
    if not os.path.isdir(models_dir):
        print(f"Error: {models_dir} not found", file=sys.stderr)
        sys.exit(1)

    types = parse_models(models_dir)
    main_config = types.get("MainConfig", [])
    if not main_config:
        print("Error: MainConfig class not found in SVM models", file=sys.stderr)
        sys.exit(1)

    fields = {}
    for fname, ftype in main_config:
        if fname == "PresetNotes":
            continue
        for path, ctype in flatten(types, ftype):
            fields[path] = ctype
    return fields


def get_quma_fields(metadata_path):
    """Extract existing field paths from metadata.rs."""
    with open(metadata_path) as f:
        content = f.read()
    return set(re.findall(r'path:\s*"([^"]+)"', content))


def label_from_path(path):
    """Derive a human label from a dotted field path."""
    name = path.rsplit(".", 1)[-1]
    # Insert spaces before capitals: "MinKills" -> "Min Kills"
    label = re.sub(r"(?<=[a-z])(?=[A-Z])", " ", name)
    # Handle acronyms: "BTRWoodsChance" -> "BTR Woods Chance"
    label = re.sub(r"(?<=[A-Z])(?=[A-Z][a-z])", " ", label)
    # Handle underscores
    label = label.replace("_", " ")
    return label


def subgroup_from_path(path):
    """Derive subgroup from the parent struct in the path."""
    parts = path.split(".")
    if len(parts) >= 2:
        return parts[0]
    return None


def input_type_rust(ctype):
    """Generate Rust InputType from C# type."""
    if ctype == "bool":
        return "InputType::Toggle"
    elif ctype == "int":
        return "InputType::Integer { min: Some(0), max: None }"
    elif ctype == "double":
        return "InputType::Float { min: Some(0.0), max: None, step: Some(0.1) }"
    else:
        return "InputType::Text"


def section_for_path(path, svm_fields_by_section):
    """Determine which section a path belongs to."""
    for section, paths in svm_fields_by_section.items():
        if path in paths:
            return section
    return "unknown"


def get_svm_fields_by_section(svm_repo):
    """Get fields grouped by top-level section."""
    models_dir = os.path.join(svm_repo, "Models", "Models")
    types = parse_models(models_dir)
    main_config = types.get("MainConfig", [])
    sections = {}
    for fname, ftype in main_config:
        if fname == "PresetNotes":
            continue
        section_key = fname.lower()
        paths = set()
        for path, _ in flatten(types, ftype):
            paths.add(path)
        sections[section_key] = paths
    return sections


def generate_stub(path, ctype):
    """Generate a FieldMeta Rust stub."""
    label = label_from_path(path)
    subgroup = subgroup_from_path(path)
    input_type = input_type_rust(ctype)
    subgroup_str = f'Some("{subgroup}")' if subgroup else "None"

    return f"""        FieldMeta {{
            path: "{path}",
            label: "{label}",
            description: "TODO: add description",
            input_type: {input_type},
            subgroup: {subgroup_str},
        }},"""


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <path-to-svm-repo>", file=sys.stderr)
        sys.exit(1)

    svm_repo = sys.argv[1]
    metadata_path = os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "src", "svm", "metadata.rs",
    )

    if not os.path.isfile(metadata_path):
        print(f"Error: {metadata_path} not found", file=sys.stderr)
        sys.exit(1)

    svm_fields = get_svm_fields(svm_repo)
    quma_fields = get_quma_fields(metadata_path)
    svm_by_section = get_svm_fields_by_section(svm_repo)

    missing = {p: t for p, t in svm_fields.items() if p not in quma_fields}
    stale = quma_fields - set(svm_fields.keys())

    print(f"SVM fields:  {len(svm_fields)}")
    print(f"Quma fields: {len(quma_fields)}")
    print(f"Missing:     {len(missing)}")
    print(f"Stale:       {len(stale)}")
    print()

    if stale:
        print("=== Stale fields (in Quartermaster but not in SVM) ===")
        for p in sorted(stale):
            print(f"  {p}")
        print()

    if not missing:
        print("All SVM fields are covered. Nothing to do.")
        return

    # Group missing by section
    by_section = {}
    for path in sorted(missing.keys()):
        section = section_for_path(path, svm_by_section)
        by_section.setdefault(section, []).append(path)

    print("=== Generated stubs for missing fields ===")
    print("// Paste these into the appropriate *_fields() function in metadata.rs")
    print()
    for section in sorted(by_section.keys()):
        paths = by_section[section]
        print(f"// --- {section} section ({len(paths)} fields) ---")
        for path in paths:
            print(generate_stub(path, missing[path]))
        print()


if __name__ == "__main__":
    main()
