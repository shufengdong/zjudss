import fs from 'node:fs'
import path from 'node:path'

import YAML from 'yaml'
import os from 'node:os'
import { execFileSync } from 'node:child_process'

const repoRoot = path.resolve(import.meta.dirname, '../../..')

function workflow(name: string) {
  return YAML.parse(fs.readFileSync(path.join(repoRoot, '.github/workflows', name), 'utf8'))
}

function commands(job: Record<string, unknown>) {
  return ((job.steps as Array<{ run?: string }> | undefined) ?? [])
    .map((step) => step.run ?? '')
    .join('\n')
}

describe('candidate release workflows', () => {
  it('creates a public test seed for CI and a private exact-byte secret seed for release', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'zjudss-seed-'))
    try {
      fs.mkdirSync(path.join(directory, 'backend/resources'), { recursive: true })
      fs.mkdirSync(path.join(directory, 'integration-tests/fixtures'), { recursive: true })
      const fixture = fs.readFileSync(path.join(repoRoot, 'integration-tests/fixtures/init_users.csv'))
      fs.writeFileSync(path.join(directory, 'integration-tests/fixtures/init_users.csv'), fixture)
      const ci = workflow('ci.yml').jobs['pr-gate'].steps.find((step: { name?: string }) => step.name === 'Prepare test users')
      const release = workflow('release-integration.yml').jobs['release-gate'].steps.find((step: { name?: string }) => step.name === 'Prepare release users')
      expect(ci).toBeDefined()
      expect(release).toBeDefined()
      execFileSync('bash', ['-euc', ci.run], { cwd: directory })
      const output = path.join(directory, 'backend/resources/init_users.csv')
      expect(fs.readFileSync(output)).toEqual(fixture)
      const secret = 'ID,NAME\nalice,$(touch SHOULD_NOT_EXIST)\n'
      execFileSync('bash', ['-euc', release.run], { cwd: directory, env: { ...process.env, INIT_USERS_CSV: secret } })
      expect(fs.readFileSync(output, 'utf8')).toBe(secret)
      expect(fs.statSync(output).mode & 0o777).toBe(0o600)
      expect(fs.existsSync(path.join(directory, 'SHOULD_NOT_EXIST'))).toBe(false)
      expect(() => execFileSync('bash', ['-euc', release.run], { cwd: directory, env: { ...process.env, INIT_USERS_CSV: '' }, stdio: 'pipe' })).toThrow()
    } finally { fs.rmSync(directory, { recursive: true, force: true }) }
  })

  it('runs L0 L1 L2 and the strict PR catalog gate', () => {
    const ci = workflow('ci.yml')
    const job = ci.jobs['pr-gate']
    const run = commands(job)

    expect(ci.on).toHaveProperty('pull_request')
    expect(ci.on).toHaveProperty('workflow_call')
    expect(run).toContain('cargo test --features e2e-test')
    expect(run).toContain('npm run test:e2e')
    expect(run).toContain('./scripts/check-functional-coverage --require-pr')
    expect(JSON.stringify(job.steps)).toContain('actions/upload-artifact@v4')
    expect(JSON.stringify(job.steps)).not.toContain('continue-on-error')
  })

  it('builds the current default eig-rs branch and runs the strict release gate without pins', () => {
    const release = workflow('release-integration.yml')
    const job = release.jobs['release-gate']
    const run = commands(job)

    expect(release.jobs['pr-gate'].uses).toBe('./.github/workflows/ci.yml')
    expect(job.needs).toBe('pr-gate')
    expect(job.environment).toBe('candidate-release')
    expect(job.env.LLM_PROVIDER).toContain('vars.LLM_PROVIDER')
    expect(job.env.LLM_MODEL).toContain('vars.LLM_MODEL')
    expect(job.env.DEEPSEEK_API_KEY).toContain('secrets.DEEPSEEK_API_KEY')
    expect(job.env.BAILIAN_API_KEY).toContain('secrets.BAILIAN_API_KEY')
    expect(job.env.BAILIAN_BASE_URL).toContain('vars.BAILIAN_BASE_URL')
    expect(release.on.workflow_dispatch?.inputs).toBeUndefined()
    expect(job.env.EXPECTED_TENSOREVAL_SHA256).toBeUndefined()
    expect(JSON.stringify(job.steps)).not.toContain('actions/download-artifact@v4')
    const engineCheckout = job.steps.find((step: any) => step.name === 'Checkout current eig-rs')
    expect(engineCheckout.uses).toBe('actions/checkout@v4')
    expect(engineCheckout.with.repository).toBe('shufengdong/eig-rs')
    expect(engineCheckout.with.submodules).toBe('recursive')
    expect(engineCheckout.with.ref).toBeUndefined()
    expect(run).toContain('cargo build --manifest-path target/eig-rs/Cargo.toml --release -p mems --bin tensoreval --no-default-features --features tensoreval-cli,with_dsa_mumps')
    expect(run).not.toContain('EXPECTED_TENSOREVAL_SHA256')
    expect(run).toContain('./scripts/test-release-integration')
    expect(run).toContain('./scripts/check-functional-coverage --require-release')
    expect(JSON.stringify(job.steps)).not.toContain('continue-on-error')
  })

  it('requires successful projection evidence for every published bundle case', () => {
    const gate = fs.readFileSync(path.join(repoRoot, 'scripts/test-release-integration'), 'utf8')

    expect(gate).toContain('projectedExamples')
    expect(gate).toContain('projectionFailures')
    expect(gate).toContain('caseProjections')
    expect(gate).toContain('duplicate_projection_evidence')
  })

  it('validates release credentials for the selected LLM provider', () => {
    const sparrowzz = fs.mkdtempSync(path.join(os.tmpdir(), 'zjudss-sparrowzz-'))
    try {
      try {
        execFileSync('sh', [path.join(repoRoot, 'scripts/test-release-integration')], {
          cwd: repoRoot,
          env: {
            ...process.env,
            ZJUDSS_SKIP_DOTENV: '1',
            LLM_PROVIDER: 'bailian',
            LLM_MODEL: 'qwen3.6-flash',
            BAILIAN_API_KEY: 'bailian-test-secret',
            BAILIAN_BASE_URL: '',
            DEEPSEEK_API_KEY: '',
            TENSOREVAL_BIN: '/bin/true',
            SPARROWZZ_DIR: sparrowzz,
          },
          stdio: 'pipe',
        })
        throw new Error('release validation unexpectedly passed')
      } catch (error: any) {
        const stderr = error.stderr?.toString() ?? ''
        expect(stderr).toContain('release_integration_missing:BAILIAN_BASE_URL')
        expect(stderr).not.toContain('release_integration_missing:DEEPSEEK_API_KEY')
        expect(stderr).not.toContain('bailian-test-secret')
      }
    } finally {
      fs.rmSync(sparrowzz, { recursive: true, force: true })
    }
  })
})


describe('local SSH deployment', () => {
  it.each(['success', 'build failure', 'upload failure', 'health failure', 'interrupt stop', 'disconnect'])(
    'builds locally and preserves runtime files on %s', (scenario) => {
      const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'zjudss-ssh-'))
      const target = path.join(directory, 'target with spaces'), tools = path.join(directory, 'tools')
      try {
        for (const name of ['scripts', 'backend/resources', 'frontend', 'tools', 'target with spaces/www']) {
          fs.mkdirSync(path.join(directory, name), { recursive: true })
        }
        fs.copyFileSync(path.join(repoRoot, 'scripts/deploy'), path.join(directory, 'scripts/deploy'))
        fs.writeFileSync(path.join(directory, 'backend/resources/init_users.csv'), 'ID,NAME\nalice,Alice\n')
        for (const [name, body] of Object.entries({
          uname: 'case ${1:-} in -s) printf Linux;; -m) printf x86_64;; *) exit 1;; esac',
          realpath: 'path=${!#}; [[ -e $path ]] || exit 1; printf "%s" "$path"',
          flock: 'exit 0',
          cargo: '[[ $DEPLOY_TEST_SCENARIO != "build failure" ]] || exit 1\nmkdir -p target/release\nprintf new > target/release/zjudss',
          npm: 'if [[ $1 == run ]]; then mkdir -p dist; printf "%s|%s" "$VITE_BASE_PATH" "$VITE_API_BASE_PATH" > dist/index.html; fi',
          ssh: 'printf "ssh\\n" >> "$DEPLOY_TEST_TRACE"\nif [[ $DEPLOY_TEST_SCENARIO == "upload failure" && ${!#} == *"tar -xzf"* ]]; then exit 1; fi\nexec bash -c "${!#}"',
          sudo: 'if [[ ${1:-} == -v ]]; then exit 0; fi\nexec "$@"',
          systemctl: 'printf "%s\\n" "$*" >> "$DEPLOY_TEST_TRACE"\ncase $1 in stop) rm -f "$DEPLOY_TEST_ACTIVE"; if [[ ( $DEPLOY_TEST_SCENARIO == "interrupt stop" || $DEPLOY_TEST_SCENARIO == disconnect ) && ! -e $DEPLOY_TEST_TRACE.signal ]]; then touch "$DEPLOY_TEST_TRACE.signal"; if [[ $DEPLOY_TEST_SCENARIO == disconnect ]]; then kill -HUP "$PPID"; else kill -TERM "$PPID"; fi; fi;; start) touch "$DEPLOY_TEST_ACTIVE";; is-active) test -f "$DEPLOY_TEST_ACTIVE";; esac',
          curl: 'if [[ $DEPLOY_TEST_SCENARIO == "health failure" && $(cat "$DEPLOY_DIR/zjudss") == new ]]; then exit 1; fi\nprintf ok',
        })) {
          fs.writeFileSync(path.join(tools, name), `#!/bin/bash\nset -eu\n${body}\n`, { mode: 0o755 })
        }
        fs.writeFileSync(path.join(target, 'zjudss'), 'old')
        fs.writeFileSync(path.join(target, 'www/index.html'), 'old-html')
        fs.writeFileSync(path.join(target, 'www/obsolete.js'), 'obsolete')
        const preserved = { '.env': 'runtime-secret', 'app.db': 'database', tensoreval: 'engine', 'data/result.parquet': 'result', 'sparrowzz/catalog': 'resources' }
        for (const [name, value] of Object.entries(preserved)) {
          fs.mkdirSync(path.dirname(path.join(target, name)), { recursive: true })
          fs.writeFileSync(path.join(target, name), value)
        }
        const trace = path.join(directory, 'trace')
        const env = { ...process.env, PATH: `${tools}:${process.env.PATH}`, DEPLOY_DIR: target, HEALTH_TIMEOUT: '1',
          DEPLOY_TEST_SCENARIO: scenario, DEPLOY_TEST_TRACE: trace, DEPLOY_TEST_ACTIVE: path.join(directory, 'active') }
        fs.writeFileSync(env.DEPLOY_TEST_ACTIVE, '')
        const run = () => execFileSync('bash', [path.join(directory, 'scripts/deploy'), 'fixture-host'], { cwd: '/', env, stdio: 'pipe' })
        if (scenario === 'success') run()
        else expect(run).toThrow()
        expect(fs.readFileSync(path.join(target, 'zjudss'), 'utf8')).toBe(scenario === 'success' ? 'new' : 'old')
        expect(fs.readFileSync(path.join(target, 'www/index.html'), 'utf8')).toBe(scenario === 'success' ? '/zjudss/|/zjudss/api' : 'old-html')
        expect(fs.existsSync(path.join(target, 'www/obsolete.js'))).toBe(scenario !== 'success')
        if (scenario === 'success') {
          expect(fs.statSync(path.join(target, 'zjudss')).mode & 0o777).toBe(0o755)
          expect(fs.statSync(path.join(target, 'www')).mode & 0o777).toBe(0o755)
          expect(fs.statSync(path.join(target, 'www/index.html')).mode & 0o777).toBe(0o644)
        }
        for (const [name, value] of Object.entries(preserved)) expect(fs.readFileSync(path.join(target, name), 'utf8')).toBe(value)
        expect(fs.readdirSync(target).some(name => name.startsWith('.upload.'))).toBe(false)
        expect(fs.existsSync(path.join(target, 'init_users.csv'))).toBe(false)
        expect(fs.existsSync(env.DEPLOY_TEST_ACTIVE)).toBe(true)
        if (scenario === 'build failure') expect(fs.existsSync(trace)).toBe(false)
        if (scenario === 'health failure') expect(fs.readFileSync(trace, 'utf8').match(/^start /gm)).toHaveLength(2)
      } finally { fs.rmSync(directory, { recursive: true, force: true }) }
    }, 20_000,
  )
})
