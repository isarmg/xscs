import { t } from '@xcss/web/admin-ui/i18n';
import type { ApplicationSpec, Command, ConfigFieldDefinition } from './api';

// Rust str::trim uses Unicode White_Space, unlike JavaScript trim (NEL/FEFF).
const surroundingWhitespace = /^\p{White_Space}|\p{White_Space}$/u;
const controls = /[\u0000-\u001f\u007f-\u009f\ud800-\udfff]/u;
export function protocolTextError(value:string, maximumBytes:number, required=false):string|null {
  if (!value) return required ? t('此项不能为空。','This field is required.') : null;
  if (new TextEncoder().encode(value).length > maximumBytes) return t('最多允许 {0} 个 UTF-8 字节。','Use at most {0} UTF-8 bytes.',[String(maximumBytes)]);
  if (surroundingWhitespace.test(value)) return t('不能包含首尾空白字符；输入会保持原样。','Remove surrounding whitespace; the input is preserved unchanged.');
  if (controls.test(value)) return t('不能包含控制字符或无效 Unicode 字符。','Control characters and invalid Unicode are not allowed.');
  return null;
}
export function configurationFieldError(field:ConfigFieldDefinition, value:string):string|null {
  if (!value) return null; // Empty means removing an explicit value.
  if (field.kind === 'text') {
    if (surroundingWhitespace.test(value) || controls.test(value) || /[#=\[\]]/.test(value)) return t('不能包含首尾空白、控制字符或 # = [ ]。','Surrounding whitespace, control characters and # = [ ] are not allowed.');
    if (field.maximum_length !== undefined && [...value].length > field.maximum_length) return t('最多允许 {0} 个字符。','Use at most {0} characters.',[String(field.maximum_length)]);
  } else if (field.kind === 'integer') {
    const number = Number(value);
    if (!Number.isSafeInteger(number) || (field.minimum !== undefined && number < field.minimum) || (field.maximum !== undefined && number > field.maximum)) return t('请输入允许范围内的整数。','Enter an integer within the allowed range.');
  } else if (!(field.kind === 'boolean' ? ['true','false'] : field.options ?? []).includes(value)) return t('请选择允许的值。','Choose an allowed value.');
  return null;
}
// Blank lines are separators. Nonempty command text is never trimmed or changed.
export const detachedCommands = (value:string) => value.split('\n').filter(line => line !== '');
export type ApplicationErrors = Record<string,string>;
export function applicationErrors(value:ApplicationSpec, hostCommandsAllowed=true):ApplicationErrors {
  const errors:ApplicationErrors = {};
  for (const key of ['name','output','cmd','working-dir','image-path'] as const) {
    const error = protocolTextError(value[key],key === 'name' ? 128 : 4096,key === 'name');
    if (error) errors[key] = error;
  }
  if (!Number.isInteger(value['exit-timeout']) || value['exit-timeout'] < 0 || value['exit-timeout'] > 3600) errors['exit-timeout'] = t('退出等待须为 0–3600 秒的整数。','Exit timeout must be an integer from 0 to 3600 seconds.');
  if (value['prep-cmd'].length > 16) errors['prep-cmd'] = t('最多允许 16 条准备命令。','Use at most 16 preparation commands.');
  value['prep-cmd'].forEach((command,index) => {
    for (const key of ['do','undo'] as const) { const error = protocolTextError(command[key],4096); if (error) errors[`prep-${index}-${key}`] = error; }
    if (!command.do && !command.undo) errors[`prep-${index}-do`] = t('执行或撤销命令至少填写一项。','Enter a Do or Undo command.');
  });
  if (value.detached.length > 16) errors.detached = t('最多允许 16 条分离命令。','Use at most 16 detached commands.');
  value.detached.forEach((command,index) => { const error = protocolTextError(command,4096,true); if (error) errors.detached ??= t('第 {0} 条分离命令：{1}','Detached command {0}: {1}',[String(index+1),error]); });
  if (!hostCommandsAllowed && (value.output || value.cmd || value['working-dir'] || value['exclude-global-prep-cmd'] || value.elevated || value['prep-cmd'].length || value.detached.length)) errors.permission = t('客户端未授权应用主机命令。','The client has not allowed application host commands.');
  return errors;
}
export function applicationTaskSizeError(command:Extract<Command,{kind:'save_application'}>):string|null {
  // IDs have the fixed canonical UUID/op_UUID wire length. Include the complete
  // Task envelope so this matches Task::fingerprint's 64KiB-minus1024 limit.
  const uuid = '00000000-0000-0000-0000-000000000000';
  const task = { protocol:'xscs-task/1', operation_id:`op_${uuid}`, binding:{manager_id:uuid,device_id:uuid,installation_id:uuid}, permission:'manage_applications', command };
  return new TextEncoder().encode(JSON.stringify(task)).length > 64*1024-1024 ? t('应用配置总大小超过任务上限，请减少命令内容。','The application exceeds the task size limit. Reduce its command content.') : null;
}
