import { createAdministratorApiClient } from "@xcss/web/admin-web";
export const adminApi = createAdministratorApiClient();
export const CURRENT_API_PREFIX = "/api/v1/rustdesk";
export type Diagnostics = { backend:"rustdesk";execution_mode:"embedded";integration_state:"awaiting_integration"|"ready"|"unavailable";rustdesk_version:string|null };
export type Capabilities = { protocol:"xscs-management/2";client_version:string;os:string;backend:"rustdesk";execution_mode:"embedded";diagnostics:true };
export type DeviceInfo = { id:string;name:string;registered:boolean;pairing_pending:boolean;revoked:boolean;client_online:boolean;diagnostics:Diagnostics|null;capabilities:Capabilities|null;last_seen_at_micros:number|null };
export type Ticket = { device:DeviceInfo;manager_id:string;token:string };
export type Report = {kind:"diagnostics_read";snapshot:Diagnostics}|{kind:"rejected"|"unknown";reason:string};
export type Operation = {operation_id:string;device_id:string;action:string;state:"pending"|"running"|"succeeded"|"failed"|"unknown"|"dead_letter"|"resolved";created_at_server:string;result:Report|null;reconciliation:Report|null};
export type OperationPage = {operations:Operation[];next_cursor:string|null;previous_cursor:string|null};
export function record(v:unknown):v is Record<string,unknown>{return typeof v==="object"&&v!==null&&!Array.isArray(v)}
const optionalString=(v:unknown):v is string|null=>v===null||typeof v==="string";
export function isDiagnostics(v:unknown):v is Diagnostics{return record(v)&&v.backend==="rustdesk"&&v.execution_mode==="embedded"&&["awaiting_integration","ready","unavailable"].includes(String(v.integration_state))&&optionalString(v.rustdesk_version)&&(v.integration_state!=="ready"||!!v.rustdesk_version)&&(v.integration_state!=="awaiting_integration"||v.rustdesk_version===null)}
function isCapabilities(v:unknown):v is Capabilities{return record(v)&&v.protocol==="xscs-management/2"&&typeof v.client_version==="string"&&typeof v.os==="string"&&v.backend==="rustdesk"&&v.execution_mode==="embedded"&&v.diagnostics===true}
export function isDevice(v:unknown):v is DeviceInfo{return record(v)&&typeof v.id==="string"&&typeof v.name==="string"&&[v.registered,v.pairing_pending,v.revoked,v.client_online].every(b=>typeof b==="boolean")&&(v.diagnostics===null||isDiagnostics(v.diagnostics))&&(v.capabilities===null||isCapabilities(v.capabilities))&&(v.last_seen_at_micros===null||typeof v.last_seen_at_micros==="number")}
export function isDevices(v:unknown):v is DeviceInfo[]{return Array.isArray(v)&&v.every(isDevice)}
export function isTicket(v:unknown):v is Ticket{return record(v)&&isDevice(v.device)&&typeof v.manager_id==="string"&&typeof v.token==="string"&&/^[a-z0-9]{36}$/.test(v.token)}
function isReport(v:unknown):v is Report{return record(v)&&((v.kind==="diagnostics_read"&&isDiagnostics(v.snapshot))||(["rejected","unknown"].includes(String(v.kind))&&typeof v.reason==="string"))}
export function isOperation(v:unknown):v is Operation{return record(v)&&typeof v.operation_id==="string"&&typeof v.device_id==="string"&&typeof v.action==="string"&&["pending","running","succeeded","failed","unknown","dead_letter","resolved"].includes(String(v.state))&&typeof v.created_at_server==="string"&&(v.result===null||isReport(v.result))&&(v.reconciliation===null||isReport(v.reconciliation))}
export function isOperationPage(v:unknown):v is OperationPage{return record(v)&&Array.isArray(v.operations)&&v.operations.every(isOperation)&&optionalString(v.next_cursor)&&optionalString(v.previous_cursor)}
export type ClientAuthorization={manager_id:string;device_id:string;authorization_code:string};
export function isClientAuthorization(v:unknown):v is ClientAuthorization{return record(v)&&typeof v.manager_id==="string"&&typeof v.device_id==="string"&&typeof v.authorization_code==="string"&&/^[a-z0-9]{36}$/.test(v.authorization_code)}
