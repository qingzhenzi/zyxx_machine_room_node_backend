#!/usr/bin/env python3
"""从 docs/openapi.json 生成 src/api/schema.ts。

刷新规范：
  curl -s http://127.0.0.1:8080/api-docs/openapi.json -o docs/openapi.json
  python3 docs/openapi-types.py
  ./node_modules/.bin/oxfmt src/api/schema.ts   # 生成物按项目格式再格式化一次
"""
from __future__ import annotations

import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SPEC = ROOT / 'docs' / 'openapi.json'
OUT = ROOT / 'src' / 'api' / 'schema.ts'


def literal(value: object) -> str:
    if value is None:
        return 'null'
    if isinstance(value, bool):
        return 'true' if value else 'false'
    if isinstance(value, (int, float)):
        return str(value)
    return "'" + str(value).replace("'", "\\'") + "'"


def ts_type(schema: dict) -> str:
    if not schema:
        return 'unknown'
    if '$ref' in schema:
        return schema['$ref'].split('/')[-1]
    if 'oneOf' in schema:
        return ' | '.join(ts_type(item) for item in schema['oneOf'])
    if 'allOf' in schema:
        return ' & '.join(ts_type(item) for item in schema['allOf'])
    if 'enum' in schema:
        return ' | '.join(literal(item) for item in schema['enum'])
    kind = schema.get('type')
    if isinstance(kind, list):
        return ' | '.join(ts_type({**schema, 'type': item}) for item in kind)
    if kind == 'array':
        return ts_type(schema.get('items', {})) + '[]'
    if kind == 'object':
        properties = schema.get('properties')
        if properties:
            required = set(schema.get('required', []))
            fields = '; '.join(
                f'{name}{"?" if name not in required else ""}: {ts_type(field)}'
                for name, field in properties.items()
            )
            return '{ ' + fields + ' }'
        return 'Record<string, unknown>'
    if kind in ('integer', 'number'):
        return 'number'
    if kind == 'boolean':
        return 'boolean'
    if kind == 'string':
        return 'string'
    if kind == 'null':
        return 'null'
    return 'unknown'


def main() -> None:
    spec = json.loads(SPEC.read_text())
    schemas = spec['components']['schemas']
    lines = [
        '// 本文件由 docs/openapi.json 生成，勿手改。',
        '// 重新生成：python3 docs/openapi-types.py',
        '',
    ]
    for name, schema in schemas.items():
        if 'enum' in schema and 'properties' not in schema:
            union = ' | '.join(literal(item) for item in schema['enum'])
            lines += [f'export type {name} = {union}', '']
    for name, schema in schemas.items():
        if 'enum' in schema and 'properties' not in schema:
            continue
        if 'allOf' in schema:
            parts = schema['allOf']
            base = next((part for part in parts if '$ref' in part), None)
            body = []
            for part in parts:
                required = set(part.get('required', []))
                body += [
                    f'  {field}{"?" if field not in required else ""}: {ts_type(sub)}'
                    for field, sub in part.get('properties', {}).items()
                ]
            head = f'export interface {name} extends {base["$ref"].split("/")[-1]} {{' if base else f'export interface {name} {{'
            lines += [head, *body, '}', '']
            continue
        required = set(schema.get('required', []))
        lines.append(f'export interface {name} {{')
        lines += [
            f'  {field}{"?" if field not in required else ""}: {ts_type(sub)}'
            for field, sub in schema.get('properties', {}).items()
        ]
        lines += ['}', '']
    OUT.write_text('\n'.join(lines))
    print(f'wrote {OUT} ({len(lines)} lines)')


if __name__ == '__main__':
    main()
