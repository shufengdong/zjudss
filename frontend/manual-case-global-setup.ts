import { validateManualCaseEnvironment } from './manual-case-guard'

export default async function guardManualCaseExecution() {
  validateManualCaseEnvironment(process.env)
}
