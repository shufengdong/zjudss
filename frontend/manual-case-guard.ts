import { sep, resolve } from 'node:path'

export function validateManualCaseEnvironment(env: NodeJS.ProcessEnv): string {
  if (env.MANUAL_CASE_OPT_IN !== '1') throw new Error('manual_case_explicit_opt_in_required')
  if (!env.MANUAL_CASE_RUN_ROOT) throw new Error('manual_case_run_root_required')

  const repositoryRoot = resolve(import.meta.dirname, '..')
  const reportRoot = resolve(repositoryRoot, 'target/manual-case-journeys')
  const runRoot = resolve(env.MANUAL_CASE_RUN_ROOT)
  if (!runRoot.startsWith(`${reportRoot}${sep}`)
    || !runRoot.slice(reportRoot.length + 1).startsWith('run.')) {
    throw new Error('manual_case_run_root_invalid')
  }
  if (env.DATABASE_URL !== `sqlite://${runRoot}/manual-cases.db`) {
    throw new Error('manual_case_database_url_invalid')
  }
  if (!env.DATA_DIR || !resolve(env.DATA_DIR).startsWith(`${runRoot}${sep}`)) {
    throw new Error('manual_case_data_dir_invalid')
  }
  return runRoot
}
