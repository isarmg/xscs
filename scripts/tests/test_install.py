import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FAKE_BINARY = '''#!/usr/bin/env python3
import json,os,pathlib,sys
root=pathlib.Path(os.environ['XSCS_INSTALL_ROOT'])
config=json.loads(pathlib.Path(sys.argv[2]).read_text())
data=root/config.get('data_dir','/var/lib/isarmg/xscs').lstrip('/')
args=sys.argv[3:]
if args[:1]==['--data-dir']:
    data=pathlib.Path(args[1]);args=args[2:]
with (root/'cli-actions').open('a') as out: out.write(' '.join(args)+'\\n')
if args[:2]==['config','validate']: sys.exit(0)
if args[0]=='init':
    assert sys.stdin.read()=='correct horse battery\\n'
    assert args[1:] in (['--username','alice'], ['--username','admin'])
    for name,content in [('xscs.sqlite3',b'database'),('secrets.json',b'stable key')]:
        with (data/name).open('xb') as f: f.write(content)
elif args[0]=='check':
    if (root/'fail-check').exists(): sys.exit(1)
    assert (data/'secrets.json').read_bytes()==b'stable key'
    assert (data/'xscs.sqlite3').read_bytes()==b'database'
else: sys.exit(2)
'''

class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.temp = Path(self.temporary.name)
        self.host = self.temp / 'host'; self.host.mkdir()
        self.release = self.temp / 'release'
        for directory in ('bin', 'config', 'systemd'):
            (self.release / directory).mkdir(parents=True)
        self.binary = self.release / 'bin/xscs'
        self.binary.write_text(FAKE_BINARY); self.binary.chmod(0o755)
        shutil.copy2(ROOT / 'deploy/install.sh', self.release / 'install.sh')
        shutil.copy2(ROOT / 'deploy/xscs.service', self.release / 'systemd/xscs.service')
        self.chosen = self.temp / 'chosen.json'
        self.chosen.write_text(json.dumps({'data_dir':'/srv/custom state','log_level':'warn'}))
        self.env = dict(os.environ, XSCS_INSTALL_TEST='1', XSCS_INSTALL_ROOT=str(self.host))
        self.config = self.host / 'etc/isarmg/xscs/config.json'
        self.state = self.host / 'srv/custom state'
    def tearDown(self): self.temporary.cleanup()
    def invoke(self, *args):
        return subprocess.run(['bash',str(self.release/'install.sh'),*args], input='correct horse battery\n',env=self.env,capture_output=True,text=True)
    def setup_installation(self):
        result=self.invoke('setup',str(self.chosen),'alice')
        self.assertEqual(result.returncode,0,result.stderr)
    def test_setup_initializes_selected_private_root_without_starting(self):
        self.setup_installation()
        self.assertEqual(self.config.read_bytes(),self.chosen.read_bytes())
        self.assertEqual(self.config.stat().st_mode & 0o777,0o600)
        for path in [self.config.parent,self.state]: self.assertEqual(path.stat().st_mode & 0o777,0o700)
        self.assertEqual((self.host/'cli-actions').read_text().splitlines(),['config validate','init --username alice','check'])
        self.assertEqual((self.host/'service-actions').read_text().splitlines(),['daemon-reload'])
        self.assertFalse((self.host/'var/lib/isarmg/xscs').exists())
        self.assertFalse((self.host/'run/isarmg/xscs').exists())
        dropin=(self.host/'etc/systemd/system/xscs.service.d/state.conf').read_text()
        self.assertIn('ReadWritePaths="/srv/custom state"',dropin)
        self.assertFalse(self.invoke('setup',str(self.chosen),'alice').returncode==0)
    def test_setup_rejects_installation_directory_overlap_before_writing(self):
        for path in ['/opt/isarmg/xscs/bin', '/etc/isarmg/xscs/state', '//opt/isarmg/xscs', '/run/isarmg/xscs/data']:
            self.chosen.write_text(json.dumps({'data_dir':path}))
            result=self.invoke('setup',str(self.chosen),'alice')
            self.assertNotEqual(result.returncode,0)
            self.assertFalse(self.config.exists())
    def test_setup_defaults_username_to_admin(self):
        result=self.invoke('setup',str(self.chosen))
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('init --username admin',(self.host/'cli-actions').read_text())
    def test_fixture_rejects_an_alias_of_real_root_before_writing(self):
        self.env['XSCS_INSTALL_ROOT']='/tmp/..'
        result=self.invoke('setup',str(self.chosen))
        self.assertNotEqual(result.returncode,0)
        self.assertIn('canonical filesystem root',result.stderr)
    def test_upgrade_preserves_state_and_restarts_only_previously_active(self):
        for active in [False,True]:
            with self.subTest(active=active):
                if not self.config.exists(): self.setup_installation()
                marker=self.host/'service-active'
                if active: marker.touch()
                paths=[self.config,self.state/'secrets.json',self.state/'xscs.sqlite3']
                before={p:(p.read_bytes(),p.stat().st_mtime_ns) for p in paths}
                self.binary.write_text(FAKE_BINARY+'\n# next version\n')
                (self.host/'service-actions').write_text('');(self.host/'cli-actions').write_text('')
                result=self.invoke('upgrade');self.assertEqual(result.returncode,0,result.stderr)
                self.assertEqual(before,{p:(p.read_bytes(),p.stat().st_mtime_ns) for p in paths})
                actions=(self.host/'service-actions').read_text().splitlines()
                self.assertEqual('stop xscs.service' in actions,active)
                self.assertEqual('start xscs.service' in actions,active)
                self.assertNotIn('init',(self.host/'cli-actions').read_text())
    def test_failed_upgrade_check_does_not_restart_service(self):
        self.setup_installation();(self.host/'service-active').touch();(self.host/'fail-check').touch()
        (self.host/'service-actions').write_text('')
        result=self.invoke('upgrade');self.assertNotEqual(result.returncode,0)
        actions=(self.host/'service-actions').read_text()
        self.assertIn('stop xscs.service',actions);self.assertNotIn('start xscs.service',actions)
    def test_systemd_path_escaping_and_no_runtime_directory(self):
        path='/srv/state "quoted" %percent \\backslash'
        self.chosen.write_text(json.dumps({'data_dir':path}))
        self.setup_installation()
        actual=(self.host/'etc/systemd/system/xscs.service.d/state.conf').read_text()
        expected=path.replace('\\','\\\\').replace('"','\\"').replace('%','%%')
        self.assertIn('ReadWritePaths="'+expected+'"',actual)

if __name__ == '__main__': unittest.main()
