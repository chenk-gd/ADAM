#!/usr/bin/env bash
# Run a cargo command with the MSVC environment so the real link.exe is used
# (works around Git Bash's /usr/bin/link shadowing MSVC link.exe).
# Usage: ./scripts/cargo-msvc.sh check --workspace --all-targets
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VCENV_SH="$REPO_ROOT/target/vcenv.sh"
VCENV_OUT="$REPO_ROOT/target/vcenv_out.txt"
VCVARS64="C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
mkdir -p "$REPO_ROOT/target"

if [ ! -f "$VCENV_SH" ]; then
  echo "vcenv.sh not found; generating via vcvars64.bat" >&2
  VCENV_OUT_WIN="$(cygpath -w "$VCENV_OUT")"
  cat > /tmp/_dumpenv.bat <<EOF
@echo off
call "$VCVARS64" >nul 2>&1
set > "$VCENV_OUT_WIN"
EOF
  cmd.exe /c "$(cygpath -w /tmp/_dumpenv.bat)"
  python - "$VCENV_OUT" "$VCENV_SH" <<'PY'
import re, sys
src, dst = sys.argv[1], sys.argv[2]
out=[]
for line in open(src,'r',errors='replace'):
    line=line.rstrip('\r\n')
    m=re.match(r'^([A-Za-z][A-Za-z0-9_]*)=(.*)$', line)
    if not m: continue
    k,v=m.group(1),m.group(2)
    if k in ('LIB','INCLUDE','PATH','WindowsSdkDir','UCRTVersion','VCToolsInstallDir','WindowsSDKVersion','LIBPATH','FrameworkDir','FrameworkVersion','VSINSTALLDIR','VCINSTALLDIR'):
        v=v.replace("'", "'\"'\"'")
        out.append(f"export {k}='{v}'")
open(dst,'w',newline='\n').write('\n'.join(out)+'\n')
PY
fi

CARGO="$(command -v cargo)"
ORIG_PATH="$PATH"

# Resolve the MSVC link.exe bin dir BEFORE sourcing vcenv.sh: sourcing it
# overwrites PATH with the Windows path list, which hides /usr/bin and makes
# cygpath unfindable. Pre-compute MSVC_BIN while the Git Bash PATH is intact.
# Read VCToolsInstallDir out of the generated env file without polluting PATH.
MSVC_BIN=""
if grep -q '^export VCToolsInstallDir=' "$VCENV_SH"; then
  VCTOOLS_WIN="$(sed -n "s/^export VCToolsInstallDir='\(.*\)'$/\1/p" "$VCENV_SH" | head -1)"
  if [ -n "$VCTOOLS_WIN" ]; then
    MSVC_BIN="$(cygpath -u "${VCTOOLS_WIN}bin\\Hostx64\\x64")"
  fi
fi
if [ -z "$MSVC_BIN" ]; then
  echo "Could not resolve MSVC link.exe bin dir from vcenv.sh" >&2
  exit 1
fi

# shellcheck disable=SC1090
( source "$VCENV_SH"
  export PATH="$MSVC_BIN:/usr/bin:$ORIG_PATH"
  exec "$CARGO" "$@"
)
