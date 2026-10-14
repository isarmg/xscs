#!/usr/bin/env python3
"""Build the runtime and delegate checksummed package assembly to pinned XSSI."""
from __future__ import annotations
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

PRODUCT = "xscs"
TARGET = "x86_64-unknown-linux-gnu"
SOURCE = Path(__file__).resolve().parents[1]

def run(args:list[str],*,cwd:Path=SOURCE,env:dict[str,str]|None=None)->str:
    if env is None:
        env = build_environment()
    result=subprocess.run(args,cwd=cwd,env=env,stdin=subprocess.DEVNULL,check=True,stdout=subprocess.PIPE,text=True)
    return result.stdout.strip()

def build_environment()->dict[str,str]:
    environment=os.environ.copy()
    environment["GIT_NO_REPLACE_OBJECTS"]="1"
    for variable in ("XSSI_REPOSITORY_TOKEN", "XSSI_SIGNING_KEY", "XSSI_SIGNING_KEY_PEM",
                     "XSSI_PACKAGE_SIGNING_KEY", "PACKAGE_SIGNING_KEY",
                     "RELEASE_SIGNING_KEY_PEM", "XSSC_SIGNING_KEY_PEM"):
        environment.pop(variable,None)
    return environment

def verify_source_checkout(source:Path,revision:str)->None:
    if (run(["git","rev-parse","HEAD"],cwd=source)!=revision
            or run(["git","status","--porcelain=v1","--untracked-files=all"],cwd=source)):
        raise ValueError("source changed during package construction")

def private_source_checkout(destination:Path,revision:str)->Path:
    if re.fullmatch(r"[0-9a-f]{40}",revision) is None:
        raise ValueError("private build requires the exact current source revision")
    verify_source_checkout(SOURCE,revision)
    entries=run(["git","ls-tree","-r","--full-tree",revision],cwd=SOURCE).splitlines()
    if not entries or len(entries)>200000:
        raise ValueError("immutable product source entry count exceeds its bound")
    for entry in entries:
        mode,kind,_=entry.split("\t",1)[0].split()
        if mode not in ("100644","100755") or kind!="blob":
            raise ValueError("immutable product source contains unsupported paths or links")
    destination.mkdir()
    run(["git","init","--quiet"],cwd=destination)
    # Git transport writes an independent checkout, without cloning hardlinks.
    run(["git","fetch","--quiet","--no-tags","--depth=1",str(SOURCE),revision],cwd=destination)
    run(["git","checkout","--quiet","--detach","FETCH_HEAD"],cwd=destination)
    verify_source_checkout(destination,revision)
    return destination

def installer_fetch_environment()->dict[str,str]:
    environment=build_environment()
    environment["GIT_TERMINAL_PROMPT"]="0"
    environment["GIT_TRACE_REDACT"]="1"
    for variable in ["GIT_TRACE","GIT_TRACE_CURL","GIT_CURL_VERBOSE","GIT_TRACE2","GIT_TRACE2_EVENT","GIT_TRACE2_PERF"]:
        environment[variable]="0"
    return environment

def pinned_installer(source:Path)->tuple[str,str]:
    raw=(source/"config/xssi.lock").read_text(encoding="ascii")
    matched=re.fullmatch(r"git_rev=([0-9a-f]{40})\n",raw)
    if matched is None or matched[1]=="0"*40:
        raise ValueError("config/xssi.lock must contain one immutable reviewed 40-hex XSSI revision")
    return "https://github.com/isarmg/xssi.git",matched[1]

def stage_verified_server(binary:Path,destination:Path,revision:str)->None:
    # The receipt is written by the trusted builder after Cargo returns this ELF.
    receipt=json.loads(Path(str(binary)+".xssi-web-build.json").read_bytes())
    if (not isinstance(receipt,dict) or receipt.get("format")!="xssi-web-build-receipt-v1"
            or not re.fullmatch(r"[0-9a-f]{64}",receipt.get("binary_sha256", ""))
            or not isinstance(receipt.get("identity"),dict)
            or receipt["identity"].get("source_revision")!=revision
            or receipt["identity"].get("product")!=PRODUCT
            or receipt["identity"].get("target")!=TARGET):
        raise ValueError("server build receipt differs from the reviewed product source")
    created=False
    try:
        with binary.open("rb") as source,destination.open("xb") as output:
            created=True
            shutil.copyfileobj(source,output)
            output.flush();os.fsync(output.fileno())
        with destination.open("rb") as source:
            digest=hashlib.file_digest(source,"sha256").hexdigest()
        if digest!=receipt["binary_sha256"]:
            raise ValueError("staged server differs from its exact build receipt")
        destination.chmod(0o755)
    except BaseException:
        if created:destination.unlink(missing_ok=True)
        raise

def stage_original_notices(destination:Path,*,source:Path=SOURCE)->None:
    shutil.copyfile(source/"LICENSE-APACHE",destination/"LICENSE")

def main()->None:
    if len(sys.argv)!=2:
        raise SystemExit(f"usage: {Path(__file__).name} /absolute/output-directory")
    output=Path(sys.argv[1])
    if not output.is_absolute() or not output.is_dir() or output.resolve()!=output or output==SOURCE or SOURCE in output.parents:
        raise ValueError("output must be an existing absolute directory outside the source repository")
    repository,pin=pinned_installer(SOURCE)
    if run(["git","status","--porcelain=v1","--untracked-files=all"]):
        raise ValueError("official packages require a clean source tree")
    revision=run(["git","rev-parse","HEAD"])
    version=tomllib.loads((SOURCE/"Cargo.toml").read_text())["workspace"]["package"]["version"]
    tag=f"v{version}"
    if run(["git","cat-file","-t",f"refs/tags/{tag}"])!="tag" or run(["git","rev-parse",f"refs/tags/{tag}^{{commit}}"])!=revision:
        raise ValueError("official packages require the current annotated version tag at HEAD")
    epoch=int(run(["git","show","-s","--format=%ct",revision]))
    name=f"{PRODUCT}-{version}-linux-x86_64"
    archive=output/f"{name}.tar.gz"
    checksum=output/f"{name}.tar.gz.sha256"
    if archive.exists() or archive.is_symlink() or checksum.exists() or checksum.is_symlink():
        raise ValueError("refusing to overwrite any release output")
    with tempfile.TemporaryDirectory(prefix=f"{PRODUCT}-package-") as temporary:
        work=Path(temporary)
        source=private_source_checkout(work/"source",revision)
        installer=work/"xssi"
        installer.mkdir()
        run(["git","init","--quiet"],cwd=installer)
        run(["git","remote","add","origin",repository],cwd=installer)
        run(["git","fetch","--quiet","--depth=1","origin",pin],cwd=installer,env=installer_fetch_environment())
        run(["git","checkout","--quiet","--detach","FETCH_HEAD"],cwd=installer)
        if run(["git","rev-parse","HEAD"],cwd=installer)!=pin:
            raise ValueError("installer checkout differs from the reviewed pin")
        stage=work/name
        (stage/"bin").mkdir(parents=True)
        (stage/"config").mkdir()
        (stage/"share").mkdir()
        metadata=work/"metadata"
        metadata.mkdir()
        web=source/"web/dist"
        target=Path(os.environ.get("CARGO_TARGET_DIR",SOURCE/"target"))
        if not target.is_absolute():raise ValueError("CARGO_TARGET_DIR must be absolute")
        env=build_environment()
        env.update({"CARGO_TARGET_DIR":str(target),"XSSI_BUILD_METADATA_DIR":str(metadata),f"{PRODUCT.upper()}_SOURCE_REVISION":revision,"XCSS_WEB_DIST":str(web),"XSSI_BUILD_TOOLS_DIR":str(installer),"XSSI_FRONTEND_METADATA_FILE":str(metadata/"frontend.json")})
        run(["npm","ci","--prefix",str(source/"web")],cwd=source,env=env)
        builder=source/"web/node_modules/.bin/xcss-build-server"
        args=[str(builder),"--config",str(source/"xcss-web-build.json"),"--mode","release","--no-install","--dist",str(web)]
        run([*args,"--web-only"],cwd=source,env=env)
        run([*args,"--rust-only","--source-revision",revision],cwd=source,env=env)
        stage_verified_server(target/TARGET/"release"/PRODUCT,stage/"bin"/PRODUCT,revision)
        example=source/"config"/"config.json.example"
        if not example.exists():example=source/"config"/f"{PRODUCT}.json.example"
        shutil.copyfile(example,stage/"config/config.json.example")
        stage_original_notices(stage/"share",source=source)
        shutil.copyfile(source/"docs/server-release-readme.md",stage/"share/README.md")
        for suffix in ["state-contract","web-assets"]:
            shutil.copyfile(metadata/f"{PRODUCT}-{suffix}.json",stage/"share"/f"{suffix}.json")
        shutil.copytree(metadata/"contracts",stage/"share/contracts")
        env.update({"XSSI_IDENTITY_FILE":str(metadata/f"{PRODUCT}-identity.json"),"XSSI_SCHEMA_FILE":str(metadata/f"{PRODUCT}-schema.json")})
        run(["bash",str(installer/"scripts/build-package.sh"),PRODUCT,str(source),str(stage)],cwd=installer,env=env)
        # Verify every payload hash, size, mode and identity without executing candidate ELF.
        run(["python3",str(installer/"scripts/package.py"),"verify",str(stage)],cwd=installer)
        verify_source_checkout(source,revision)
        verify_source_checkout(SOURCE,revision)
        staged_archive=work/archive.name
        with staged_archive.open("wb") as archive_stream,gzip.GzipFile(filename="",fileobj=archive_stream,mode="wb",mtime=epoch) as compressed,tarfile.open(fileobj=compressed,mode="w",format=tarfile.PAX_FORMAT) as tar:
            for path in [stage,*sorted(stage.rglob("*"))]:
                if path.is_symlink() or not (path.is_file() or path.is_dir()):raise ValueError("package has an unsafe filesystem entry")
                entry=tar.gettarinfo(str(path),arcname=str(path.relative_to(work)))
                entry.uid=entry.gid=0;entry.uname=entry.gname="";entry.mtime=epoch
                if path.is_file():
                    with path.open("rb") as content:tar.addfile(entry,content)
                else:tar.addfile(entry)
        with staged_archive.open("rb") as source_stream:
            digest=hashlib.file_digest(source_stream,"sha256").hexdigest()
        # Exclusive output avoids silently replacing already published bytes.
        created_archive=False
        created_checksum=False
        try:
            with archive.open("xb") as destination,staged_archive.open("rb") as source:
                created_archive=True
                shutil.copyfileobj(source,destination);destination.flush();os.fsync(destination.fileno())
            with checksum.open("x") as destination:
                created_checksum=True
                destination.write(f"{digest}  {archive.name}\n");destination.flush();os.fsync(destination.fileno())
        except BaseException:
            if created_archive:archive.unlink(missing_ok=True)
            if created_checksum:checksum.unlink(missing_ok=True)
            raise
    print(archive)
    print(checksum)

if __name__=="__main__":main()
