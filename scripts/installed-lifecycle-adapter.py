"""XSCS assertions for XSSI's real installed lifecycle gate (no database fixtures)."""
import json
import secrets

PRODUCT = 'xscs'
SERVICES = ('xscs.service',)
ACCOUNT = 'isarmg-xscs'
RETAINED_FILES = ('state/xscs.sqlite3', 'state/secrets.json')


def configure(config, root, shared, port):
    config.update(bind=f'127.0.0.1:{port}', development=False)


def api(request, path, method='GET', body=None, csrf=None):
    headers = {'Content-Type': 'application/json'}
    if csrf:
        headers['X-CSRF-Token'] = csrf
    status, response = request(path, method, None if body is None else json.dumps(body).encode(), headers)
    if status not in (200, 201):
        raise RuntimeError('Unexpected business API status')
    return json.loads(response)


def exercise(request, session, sensitive, require):
    csrf = session['csrf_token']
    ticket = api(request, '/api/v1/rustdesk/devices', 'POST', {'name': 'Original lifecycle device', 'role': 'controlled'}, csrf)
    device = ticket['device']
    require(device['name'] == 'Original lifecycle device', 'Device creation differs')
    path = '/api/v1/rustdesk/devices/' + device['id']
    renamed = api(request, path, 'PATCH', {'name': 'Renamed lifecycle device'}, csrf)
    require(renamed['name'] == 'Renamed lifecycle device', 'Device rename differs')
    code = secrets.token_hex(18)
    sensitive.append(code)
    # Creation may return a private enrollment ticket. Never log any API body.
    authorization = api(request, path + '/authorization', 'PUT', {'authorization_code': code}, csrf)
    require(authorization['authorization_code'] == code, 'Device authorization differs')
    devices = api(request, '/api/v1/rustdesk/devices')
    require(len(devices) == 1 and devices[0]['id'] == device['id']
            and devices[0]['name'] == 'Renamed lifecycle device', 'Device list differs')
    return {'devices': devices, 'authorization': authorization, 'path': path}


def persisted(request, session, state, require):
    current = api(request, '/api/v1/auth/session')
    require(current['authenticated'] is True and current['user_id'] == session['user_id'],
            'Administrator session did not survive restart')
    require(api(request, '/api/v1/rustdesk/devices') == state['devices'], 'Devices did not survive restart')
    require(api(request, state['path'] + '/authorization') == state['authorization'],
            'Device authorization did not survive restart')


def retained(root, shared, state, require):
    require(all((root / name).is_file() for name in RETAINED_FILES),
            'Operator uninstall did not retain the database and instance secrets')
