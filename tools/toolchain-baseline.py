#!/usr/bin/env python3
"""Run identical gallery click/wheel scenarios against the historical baseline and this checkout.
The old checkout is disposable and gets counters only; no product files change.
"""
from pathlib import Path
import json, os, re, subprocess, tarfile, io
root=Path(__file__).resolve().parent.parent
before=root/'target/toolchain-baseline/before'
before.mkdir(parents=True,exist_ok=True)
baseline='18a509fa03cd2556453ea0adba9ae991e7a8800e'
archive=subprocess.check_output(['git','archive',baseline],cwd=root)
with tarfile.open(fileobj=io.BytesIO(archive)) as tar: tar.extractall(before)
quickjs=before/'third_party/quickjs'
if not quickjs.exists(): quickjs.symlink_to(root/'third_party/quickjs',target_is_directory=True)
# Counters share exactly the same boundary: scene style/text/image/recognizer
# mutations that call Tree setters. Springs and layout internals are excluded.
p=before/'crates/session/src/lib.rs'
s=p.read_text().replace('pub struct FrameTrace {','pub struct FrameTrace {\n    pub updates: u32,\n    pub sets: u32,');p.write_text(s)
p=before/'crates/quickjs/src/linked/frame.rs';s=p.read_text().replace('pub(super) struct FrameMarks {','pub(super) struct FrameMarks {\n    pub(super) updates: Cell<u32>,\n    pub(super) sets: Cell<u32>,').replace('        self.pointers.set(0);','        self.updates.set(0); self.sets.set(0);\n        self.pointers.set(0);').replace('            pointers: Cell::new(0),','            updates: Cell::new(0), sets: Cell::new(0),\n            pointers: Cell::new(0),');p.write_text(s)
p=before/'crates/quickjs/src/linked/mod.rs';s=p.read_text().replace('let trace = session::FrameTrace {','let trace = session::FrameTrace {\n                updates: inner.marks.updates.take(), sets: inner.marks.sets.take(),');s=s.replace('            let failed = unsafe {\n                qjs_frame(', '            bump(&inner.marks.updates);\n            let failed = unsafe {\n                qjs_frame(').replace('                    unsafe { qjs_layout(', '                    { bump(&inner.marks.updates); unsafe { qjs_layout(').replace('error.len()) };\n                layout_cb_ms', 'error.len()) } };\n                layout_cb_ms');p.write_text(s)
p=before/'crates/quickjs/src/linked/mutation.rs';s=p.read_text().replace('pub(super) fn apply_now(inner: &Inner, mutation: Mutation) -> Result<Applied, uicore::Error> {','pub(super) fn apply_now(inner: &Inner, mutation: Mutation) -> Result<Applied, uicore::Error> {\n    if matches!(mutation,Mutation::Style{..}|Mutation::SetText{..}|Mutation::SetImage{..}|Mutation::Recognizers{..}) {super::frame::bump(&inner.marks.sets); }');p.write_text(s)
benchmark=(root/'tools/toolchain-benchmark.rs').read_text().replace('crate::text::MonoShaper', 'uicore::MonoShaper')
benchmark+="""
    #[test]
    #[ignore = "run explicitly to collect frame samples"]
    fn toolchain_baseline_counts() {
        sample_gallery_frames();
    }
"""
p=before/'crates/quickjs/src/lib.rs';s=p.read_text();idx=s.index('    #[test]',s.index('mod tests {'));p.write_text(s[:idx]+benchmark+s[idx:])
env=os.environ.copy();env.setdefault('RUSTUP_TOOLCHAIN','1.97.0')
# Dependencies are installed against the old lock, including universal rc.17.
subprocess.run(['pnpm','install','--frozen-lockfile'],cwd=before,env=env,check=True)
subprocess.run(['node','build.mjs'],cwd=before/'apps/gallery-solid',env=env,check=True)
results={}
for label,cwd,package in [('before',before,['-p','quickjs']),('after',root,['-p','engine','--features','js'])]:
    proc=subprocess.run(['cargo','test',*package,'--lib','toolchain_baseline_counts','--','--ignored','--nocapture'],cwd=cwd,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    (root/f'target/toolchain-baseline/{label}.log').write_text(proc.stdout)
    if proc.returncode: raise SystemExit(proc.stdout[-6000:])
    results[label]={scenario:{'updates':int(updates),'sets':int(sets),'script_us':float(us)} for scenario,updates,sets,us in re.findall(r'BENCH (\w+) (\d+) (\d+) ([\d.]+)',proc.stdout)}
if any(set(results[label]) != {'click', 'scroll'} for label in ('before','after')):
    raise SystemExit('baseline requires linked QuickJS and both gallery scenarios')
results['revision']=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
out=root/'target/toolchain-baseline/results.json';out.write_text(json.dumps(results,indent=2)+'\n');print(out.read_text())
