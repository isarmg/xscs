#!/usr/bin/env python3
"""Adapter contract regressions, not substitutes for the real installed CI gate."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('adapter', Path(__file__).with_name('installed-lifecycle-adapter.py'))
adapter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


class AdapterContract(unittest.TestCase):
    def test_production_configuration(self):
        config = {'development': True}
        adapter.configure(config, Path('/root'), Path('/shared'), 12345)
        self.assertEqual(config, {'development': False, 'bind': '127.0.0.1:12345'})

    def test_business_and_persisted_session_contract(self):
        device = {'id': 'id-1', 'name': 'Original lifecycle device'}
        authorization = {}
        calls = []
        session = {'csrf_token': 'csrf', 'authenticated': True, 'user_id': 'admin-id'}

        def request(path, method, body, headers):
            calls.append((path, method))
            data = None if body is None else json.loads(body)
            if method != 'GET':
                self.assertEqual(headers['X-CSRF-Token'], session['csrf_token'])
            if path == '/api/v1/auth/session':
                result = session
            elif path.endswith('/authorization'):
                if method == 'PUT':
                    authorization.update(data)
                result = authorization
            elif method == 'POST':
                result = {'device': device}
            elif method == 'PATCH':
                device.update(data)
                result = device
            else:
                result = [device]
            return (201 if method == 'POST' else 200), json.dumps(result).encode()

        sensitive = []
        state = adapter.exercise(request, session, sensitive, require)
        adapter.persisted(request, session, state, require)
        self.assertEqual(sensitive, [authorization['authorization_code']])
        self.assertEqual(len(sensitive[0]), 36)
        self.assertIn(('/api/v1/sunshine/devices/id-1', 'PATCH'), calls)
        session['authenticated'] = False
        with self.assertRaisesRegex(RuntimeError, 'session did not survive'):
            adapter.persisted(request, session, state, require)


if __name__ == '__main__':
    unittest.main()
