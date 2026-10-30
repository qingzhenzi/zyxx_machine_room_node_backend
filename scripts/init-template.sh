#!/usr/bin/env bash
# 把模板里的 QEStudio / qestudio / qe-* 标识改成新项目。
# 用法：scripts/init-template.sh <slug> <display-name> [repo-url] [--yes] [--force]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ASSUME_YES=0
FORCE=0
ARGS=()
while (($#)); do
  case "$1" in
    --yes|-y) ASSUME_YES=1 ;;
    --force) FORCE=1 ;;
    --help|-h)
      sed -n '2,5p' "$0"
      exit 0
      ;;
    -*)
      echo "未知参数：$1" >&2
      exit 2
      ;;
    *) ARGS+=("$1") ;;
  esac
  shift
done

if ((${#ARGS[@]} < 2)); then
  echo "用法：scripts/init-template.sh <slug> <display-name> [repo-url] [--yes] [--force]" >&2
  exit 2
fi

SLUG="${ARGS[0]}"
DISPLAY="${ARGS[1]}"
REPO_URL="${ARGS[2]:-https://github.com/your-org/${SLUG}}"

if [[ ! "$SLUG" =~ ^[a-z][a-z0-9-]{1,62}$ ]]; then
  echo "slug 必须是小写字母开头、仅含小写字母/数字/短横线，长度 2-63：$SLUG" >&2
  exit 2
fi
if [[ -z "$DISPLAY" || "$DISPLAY" == *$'\n'* ]]; then
  echo "display-name 不能为空且不能包含换行" >&2
  exit 2
fi
if [[ "$REPO_URL" != http* ]]; then
  echo "repo-url 必须以 http(s):// 开头：$REPO_URL" >&2
  exit 2
fi
if ! command -v perl >/dev/null 2>&1; then
  echo "需要 perl 做文本替换" >&2
  exit 2
fi
IN_GIT=0
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  IN_GIT=1
fi
if ((IN_GIT == 1 && FORCE == 0)) && [[ -n "$(git status --porcelain)" ]]; then
  echo "工作区有未提交改动。请先提交/stash，或加 --force。" >&2
  git status --short >&2
  exit 1
fi

SLUG_UNDERSCORE="${SLUG//-/_}"
echo "将执行以下重命名："
echo "  qestudio  -> $SLUG"
echo "  QEStudio  -> $DISPLAY"
echo "  qe-*      -> $SLUG-*"
echo "  qe_*      -> ${SLUG_UNDERSCORE}_*"
echo "  repository -> $REPO_URL"
if ((ASSUME_YES == 0)); then
  read -r -p "继续？[y/N] " answer
  [[ "$answer" == "y" || "$answer" == "Y" ]] || { echo "已取消"; exit 0; }
fi

FILES=()
if ((IN_GIT == 1)); then
  while IFS= read -r -d '' file; do
    FILES+=("$file")
  done < <(
    git grep -I -l -z -E 'qestudio|QEStudio|qe-(api|auth|cache|db|domain)|qe_(api|auth|cache|db|domain)' \
      -- . ':!scripts/init-template.sh' || true
  )
else
  # 从 ZIP 下载的模板没有 .git，退化为文件系统扫描。
  while IFS= read -r -d '' file; do
    [[ "$file" == "./scripts/init-template.sh" || "$file" == "./.env" ]] && continue
    grep -Iq . "$file" || continue
    FILES+=("$file")
  done < <(find . -type f -not -path './.git/*' -not -path './target/*' -print0)
fi

if ((${#FILES[@]} == 0)); then
  echo "没有找到需要替换的标识，可能已经初始化过。" >&2
  exit 1
fi

SLUG="$SLUG" SLUG_UNDERSCORE="$SLUG_UNDERSCORE" DISPLAY="$DISPLAY" REPO_URL="$REPO_URL" \
  perl -pi -e '
    s{https://github\.com/QEStudio/backend-template}{$ENV{REPO_URL}}g;
    s{\bqe-(api|auth|cache|db|domain)\b}{$ENV{SLUG}-$1}g;
    s{\bqe_(api|auth|cache|db|domain)\b}{$ENV{SLUG_UNDERSCORE}_$1}g;
    s{qestudio}{$ENV{SLUG}}g;
    s{\bQEStudio\b}{$ENV{DISPLAY}}g;
  ' "${FILES[@]}"

echo "已替换 ${#FILES[@]} 个文件。"

if ((IN_GIT == 1)); then
  if git grep -n -E 'qestudio|QEStudio|qe-(api|auth|cache|db|domain)\b|qe_(api|auth|cache|db|domain)\b' \
    -- . ':!scripts/init-template.sh' >/dev/null 2>&1; then
    echo "警告：仍发现旧标识，请手动检查：" >&2
    git grep -n -E 'qestudio|QEStudio|qe-(api|auth|cache|db|domain)\b|qe_(api|auth|cache|db|domain)\b' \
      -- . ':!scripts/init-template.sh' >&2 || true
  fi
else
  if grep -RIn --exclude-dir=.git --exclude-dir=target --exclude=init-template.sh --exclude=.env \
    -E 'qestudio|QEStudio|qe-(api|auth|cache|db|domain)\b|qe_(api|auth|cache|db|domain)\b' . >/dev/null 2>&1; then
    echo "警告：仍发现旧标识，请手动检查：" >&2
    grep -RIn --exclude-dir=.git --exclude-dir=target --exclude=init-template.sh --exclude=.env \
      -E 'qestudio|QEStudio|qe-(api|auth|cache|db|domain)\b|qe_(api|auth|cache|db|domain)\b' . >&2 || true
  fi
fi

# 只校验 manifest 与包名，不触发完整编译；完整检查交给 make check。
cargo metadata --no-deps --format-version 1 >/dev/null

echo
echo "下一步："
echo "  1. cp .env.example .env，然后 make secret 并把输出写入 JWT_SECRET"
echo "  2. 修改 .env 里的 POSTGRES_PASSWORD、BOOTSTRAP_ADMIN_PASSWORD、CORS_ALLOWED_ORIGINS"
echo "  3. make check && make integration"
echo "  4. git diff --stat && git add -A && git commit -m 'chore: initialize from template'"
