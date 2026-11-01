import { createHash } from 'node:crypto'

export const SUPPORTED_CASE_FACADES = [
  'ds_newton_raphson_pf',
  'ds_fault_study',
  'ds_network_dynamics',
  'ds_radial_execution_plan',
  'ds_newton_raphson_execution_plan',
  'ds_harmonic_execution_plan',
] as const

export type SupportedCaseFacade = typeof SUPPORTED_CASE_FACADES[number]

export const CASE_PROMPTS: Readonly<Record<SupportedCaseFacade, string>> = {
  ds_newton_raphson_pf: '请先检查当前工作区，再严格使用 ds_newton_raphson_pf 对当前 active 模型执行 Newton-Raphson 静态潮流，参数通过 para 参数卡确认，结果选择推荐的 bus_voltage。即使模型是径向网络也不要改用 ds_radial_pf。',
  ds_fault_study: '请先检查当前工作区，再严格使用 ds_fault_study 对当前 active 模型执行短路研究，故障工况使用已上传且 schema 兼容的 fault 数据资源，para 通过参数卡确认，结果选择推荐的 fault；缺少故障数据时先要求上传或选择。',
  ds_network_dynamics: '请先检查当前工作区，再严格使用 ds_network_dynamics 对当前 active 模型执行完整网络动态研究，使用已上传且 schema 兼容的 timeline、event、tcc_curve 和 fault 数据资源，spec 与 para 通过参数卡确认，结果选择推荐的 bus_voltage；不要改用电机专用旧版门面。',
  ds_radial_execution_plan: '请先检查当前工作区，再严格使用 ds_radial_execution_plan 重放当前案例的径向执行计划。使用案例导入产生且 schema 兼容的 solution、load_shape、tcc_curve、fault、execution_epoch、execution_setting、execution_mutation、post_solve_action、execution_reference、execution_event 和 projection_diagnostic 资源，para 通过参数卡确认，结果选择推荐的 bus_voltage；不要改用 Newton 执行计划。',
  ds_newton_raphson_execution_plan: '请先检查当前工作区，再严格使用 ds_newton_raphson_execution_plan 重放当前案例的 Newton-Raphson 执行计划。使用案例导入产生且 schema 兼容的 solution、load_shape、tcc_curve、fault、execution_epoch、execution_setting、execution_mutation、post_solve_action、execution_reference、execution_event 和 projection_diagnostic 资源，para 通过参数卡确认，结果选择推荐的 bus_voltage；不要改用径向执行计划。',
  ds_harmonic_execution_plan: '请先检查当前工作区，再严格使用 ds_harmonic_execution_plan 执行当前 Run_Scan 类案例的整数或非整数频率扫描。使用案例导入产生且 schema 兼容的 load_shape、tcc_curve、fault、frequency_axis、frequency_spectrum 和 harmonic_snapshot 数据资源，solution、harmonic_plan 与 para 通过参数卡确认，并使用门面要求的当前模型和谐波快照模型表，结果选择推荐的 bus_voltage；不要退回路径型 import 或 ds_harmonic_study。',
}

export function promptForFacade(functionName: string): string {
  if (!SUPPORTED_CASE_FACADES.includes(functionName as SupportedCaseFacade)) {
    throw new Error(`unsupported_case_facade:${functionName}`)
  }
  return CASE_PROMPTS[functionName as SupportedCaseFacade]
}

export function promptSha256(prompt: string): string {
  return createHash('sha256').update(prompt).digest('hex')
}
