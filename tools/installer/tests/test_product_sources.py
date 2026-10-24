"""Product source binding prevents remote dependencies and coedited inputs."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'scripts'))
import frontend_receipt
import product_sources


def sha(value):
    return hashlib.sha256(value).hexdigest()


class ProductSourceTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='xscs-local-source-')
        self.addCleanup(temporary.cleanup)
        self.source=Path(temporary.name)/'source'
        self.web=self.source/'web'
        self.owner='xcss-web/vendor/xcss-web'
        self.directory=self.web/self.owner
        self.directory.mkdir(parents=True)
        self.files={'Cargo.toml':b'[package]\nname="xcss-web"\nversion="1.0.0"\nlicense="Apache-2.0"\n',
                    'LICENSE':b'Original Apache-2.0 fixture grant.\n',
                    'startup.js':b'export const ready=true;\n'}
        self.record={'format':1,'package':'xcss-web','version':'1.0.0',
            'origin':'https://github.com/isarmg/xcss','base_revision':'a'*40,
            'source_directory':'web/xcss-web','files':{name:sha(value) for name,value in self.files.items()}}
        for name,value in self.files.items():(self.directory/name).write_bytes(value)
        (self.directory/'SOURCE.json').write_text(json.dumps(self.record))
        subprocess.run(['git','init','--quiet',str(self.source)],check=True)
        subprocess.run(['git','-C',str(self.source),'add','.'],check=True)
        subprocess.run(['git','-C',str(self.source),'-c','user.name=Owned fixture','-c',
                        'user.email=fixture@example.invalid','commit','--quiet','-m','Record source fixture'],check=True)
        self.revision=subprocess.check_output(['git','-C',str(self.source),'rev-parse','HEAD'],text=True).strip()

    def test_runtime_uses_product_source_without_fetching_upstream(self):
        stage=self.source.parent/'stage'
        (stage/'bin').mkdir(parents=True)
        payload=b'opaque compiled Server fixture'
        (stage/'bin/xscs').write_bytes(payload)
        modules={'modules':[{'path':self.owner+'/startup.js','sha256':sha(self.files['startup.js']),
                            'files':['app.js'],'imports':[]}]}
        def target(names):
            return [{'path':'bin/xscs','sha256':sha(payload),'archive_entry':None}]
        with mock.patch.object(frontend_receipt,'download',side_effect=AssertionError('unexpected network dependency')):
            components,_=frontend_receipt.rendered_components(self.source,self.web,stage,modules,{'app.js':'1'*64},target)
        component=components[0]
        imported=(self.directory/'SOURCE.json').read_bytes()
        self.assertEqual(component['source'],{'url':'https://raw.githubusercontent.com/isarmg/xscs/'+self.revision+
            '/web/'+self.owner+'/SOURCE.json','sha256':sha(imported)})
        self.assertEqual((stage/component['licenses'][0]['path']).read_bytes(),self.files['LICENSE'])
        self.assertEqual((stage/'share/build-inputs/frontend/imports/xcss-web-SOURCE.json').read_bytes(),imported)
        self.assertIn({'path':'web/'+self.owner+'/SOURCE.json','sha256':sha(imported)},component['embedding']['source_files'])

    def test_coedited_source_and_inventory_cannot_impersonate_the_product_commit(self):
        changed=b'export const injected=true;\n'
        (self.directory/'startup.js').write_bytes(changed)
        self.record['files']['startup.js']=sha(changed)
        (self.directory/'SOURCE.json').write_text(json.dumps(self.record))
        with self.assertRaisesRegex(ValueError,'original product source revision'):
            frontend_receipt.Packages(self.web,self.source).vendor(self.owner)

    def test_runtime_inputs_require_a_tracked_product_source(self):
        (self.source/'untracked.js').write_bytes(b'untracked fixture')
        with self.assertRaisesRegex(ValueError,'declared Git commit'):
            product_sources.committed_file(self.source,self.revision,'untracked.js')
        with self.assertRaises(ValueError):
            product_sources.identity(self.directory)
    def test_installed_lifecycle_requires_the_same_owned_clean_product_commit(self):
        template=Path(__file__).with_name('installed_lifecycle.py')
        harness=self.source/'tools/installer/tests/installed_lifecycle.py'
        harness.parent.mkdir(parents=True)
        harness.write_bytes(template.read_bytes())
        adapter=self.source/'scripts/adapter.py'
        adapter.parent.mkdir()
        marker=self.source.parent/'adapter-executed'
        adapter.write_text("PRODUCT='xscs'\nACCOUNT='isarmg-xscs'\nSERVICES=('xscs.service',)\n"+
            "from pathlib import Path\nPath("+repr(str(marker))+").touch()\n")
        subprocess.run(['git','-C',str(self.source),'add','.'],check=True)
        subprocess.run(['git','-C',str(self.source),'-c','user.name=Owned fixture','-c',
            'user.email=fixture@example.invalid','commit','--quiet','-m','Record lifecycle fixtures'],check=True)
        revision=subprocess.check_output(['git','-C',str(self.source),'rev-parse','HEAD'],text=True).strip()
        # The real lifecycle job uses sudo on a runner-owned product checkout.
        if os.geteuid()==0:
            owner=self.source.stat()
            os.chown(self.source,65534,65534)
            self.addCleanup(os.chown,self.source,owner.st_uid,owner.st_gid)
        def load(file):
            spec=importlib.util.spec_from_file_location('owned_lifecycle_fixture',file)
            module=importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            return module
        current=load(harness)
        self.assertEqual(current.load_adapter(adapter,revision).PRODUCT,'xscs')
        self.assertTrue(marker.exists())
        marker.unlink()
        adapter.write_text(adapter.read_text()+'# uncommitted source\n')
        with self.assertRaisesRegex(RuntimeError,'exact clean candidate'):
            current.load_adapter(adapter,revision)
        self.assertFalse(marker.exists())
        subprocess.run(['git','-c','safe.directory='+str(self.source),'-C',str(self.source),'checkout','--','scripts/adapter.py'],check=True)
        foreign=self.source.parent/'foreign-installer/tests/installed_lifecycle.py'
        foreign.parent.mkdir(parents=True)
        foreign.write_bytes(harness.read_bytes())
        with self.assertRaisesRegex(RuntimeError,'candidate product repository'):
            load(foreign).load_adapter(adapter,revision)
        self.assertFalse(marker.exists())


class RuntimeSourceCacheTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='xscs-runtime-cache-')
        self.addCleanup(temporary.cleanup)
        self.cache=Path(temporary.name)
        self.payload=b'original pinned distribution fixture'
        self.expected=sha(self.payload)
        self.url='https://example.invalid/original.zip'

    def test_exact_cache_hit_is_verified_without_network_or_writes(self):
        archive=self.cache/self.expected
        archive.write_bytes(self.payload)
        before=archive.stat()
        with mock.patch.dict(os.environ,{'XSSI_RUNTIME_SOURCE_CACHE':str(self.cache)}), \
                mock.patch.object(frontend_receipt.urllib.request,'build_opener',side_effect=AssertionError('unexpected network')):
            self.assertEqual(frontend_receipt.download(self.url,self.expected),self.payload)
            with self.assertRaisesRegex(ValueError,'public HTTPS'):
                frontend_receipt.download('http://example.invalid/original.zip',self.expected)
        self.assertEqual(list(self.cache.iterdir()),[archive])
        self.assertEqual(archive.stat().st_mtime_ns,before.st_mtime_ns)

    def test_corrupt_unsafe_or_oversized_cache_is_rejected_without_fallback(self):
        archive=self.cache/self.expected
        target=self.cache/'original'
        with mock.patch.dict(os.environ,{'XSSI_RUNTIME_SOURCE_CACHE':str(self.cache)}), \
                mock.patch.object(frontend_receipt.urllib.request,'build_opener',side_effect=AssertionError('unexpected network')):
            archive.write_bytes(b'corrupt fixture')
            with self.assertRaisesRegex(ValueError,'cache archive SHA-256 differs'):
                frontend_receipt.download(self.url,self.expected)
            archive.unlink()
            target.write_bytes(self.payload)
            archive.symlink_to(target)
            with self.assertRaisesRegex(ValueError,'link'):
                frontend_receipt.download(self.url,self.expected)
            archive.unlink()
            os.link(target,archive)
            with self.assertRaisesRegex(ValueError,'single-link'):
                frontend_receipt.download(self.url,self.expected)
            archive.unlink()
            archive.write_bytes(self.payload)
            with mock.patch.object(frontend_receipt,'MAX_ARCHIVE',len(self.payload)-1):
                with self.assertRaisesRegex(ValueError,'bounded'):
                    frontend_receipt.download(self.url,self.expected)
            with mock.patch.dict(os.environ,{'XSSI_RUNTIME_SOURCE_CACHE':'relative-cache'}):
                with self.assertRaisesRegex(ValueError,'absolute physical directory'):
                    frontend_receipt.download(self.url,self.expected)
            directory_link=self.cache/'directory-link'
            directory_link.symlink_to(self.cache,target_is_directory=True)
            with mock.patch.dict(os.environ,{'XSSI_RUNTIME_SOURCE_CACHE':str(directory_link)}):
                with self.assertRaisesRegex(ValueError,'physical directories'):
                    frontend_receipt.download(self.url,self.expected)
            with self.assertRaisesRegex(ValueError,'SHA-256 is invalid'):
                frontend_receipt.download(self.url,'../not-a-sha')

    def test_missing_cache_unset_cache_and_unpinned_sources_use_original_downloader(self):
        for mode in ('missing','unset','unpinned'):
            with self.subTest(mode=mode), mock.patch.dict(os.environ,clear=False):
                os.environ.pop('XSSI_RUNTIME_SOURCE_CACHE',None)
                if mode!='unset':
                    os.environ['XSSI_RUNTIME_SOURCE_CACHE']=str(self.cache) if mode=='missing' else 'unused-relative-cache'
                response=io.BytesIO(self.payload)
                response.geturl=lambda:self.url
                with mock.patch.object(frontend_receipt.urllib.request,'build_opener') as factory:
                    factory.return_value.open.return_value=response
                    expected=None if mode=='unpinned' else self.expected
                    self.assertEqual(frontend_receipt.download(self.url,expected),self.payload)
                    factory.return_value.open.assert_called_once_with(self.url,timeout=60)
                    self.assertIsInstance(factory.call_args.args[0],frontend_receipt.HTTPSRedirects)


if __name__=='__main__':
    unittest.main()
