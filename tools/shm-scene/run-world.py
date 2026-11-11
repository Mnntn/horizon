"""Bounded fresh dedicated worlds, same sample source native and wasm.
Run only after the standalone optimized production-feature runner has built. No real user data.
"""
import os, pathlib, shutil, subprocess, sys, tempfile, time, threading, json
ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools'))
import lab  # noqa: E402
OUT = pathlib.Path(os.environ.get('BENCH_WORLD_OUTPUT', '/box/shared/wasm-bench'))
OUT.mkdir(parents=True, exist_ok=True)
BIN = pathlib.Path(os.environ['BENCH_BINARY'])
SAMPLE = pathlib.Path(os.environ['BENCH_SAMPLE'])
PACKAGE = pathlib.Path(os.environ['BENCH_PACKAGE'])
# Native package validated by the same loader as a launcher build.
adj = BIN.parent / 'horizon-native-mods/machine_sample'
adj.mkdir(parents=True, exist_ok=True)
for folder in ['assets', 'data', 'src']:
    shutil.copytree(SAMPLE / folder, adj / folder, dirs_exist_ok=True)
shutil.copy(SAMPLE / 'Cargo.toml', adj / 'Cargo.toml')
manifest = (SAMPLE/'horizon-mod.toml').read_text().split('\n[wasm]')[0]
# The sample's native-only demo injection has no wasm guest export. Neither
# benchmark installs it; machine logic and the rest of the source remain unchanged.
manifest += '\n[native]\ngame = "26.3"\nhorizon = "' + os.environ['BENCH_BINARY_HEAD'] + '"\nmixins = "e59da9094f0c2fdda6c627fe9eff22dec6a906f0"\n'
manifest = manifest.replace('b5d6f467dc78deda3b072e700f767adb456cd349', '1975cf5a9033a3057e439c62b07af0683254e6fb')
(adj / 'horizon-mod.toml').write_text(manifest)
shutil.copy(pathlib.Path(os.environ['BENCH_STD']), BIN.parent / 'horizon-wasm-std.wasm')

def run(mode, count):
    (ROOT / 'target').mkdir(exist_ok=True)
    home = pathlib.Path(tempfile.mkdtemp(prefix='wasm-world-', dir=ROOT / 'target'))
    env = lab.game_env(HORIZON_HOME=home/'home', HORIZON_LAUNCHER_HOME=home/'launcher', XDG_DATA_HOME=home/'data', XDG_CONFIG_HOME=home/'config')
    if mode == 'native': env['BENCH_NATIVE'] = '1'
    (home/'eula.txt').write_text('eula=true\n')
    (home/'server.properties').write_text('online-mode=false\nserver-ip=127.0.0.1\nserver-port=25578\nlevel-seed=1\nlevel-type=minecraft:flat\ngenerator-settings={"biome":"minecraft:plains","layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:dirt","height":2},{"block":"minecraft:grass_block","height":1}]}\nview-distance=3\nsimulation-distance=3\nspawn-protection=0\npause-when-empty-seconds=-1\n')
    if mode.startswith('wasm'):
        mods = home / 'horizon-mods'; mods.mkdir()
        shutil.copy(PACKAGE, mods/'machine_sample.hmod')
        (home/'horizon-mods.txt').write_text('mod.machine_sample:true\n')
    log = open(home/'benchmark.log', 'w')
    args = [str(BIN), '--resources', '/box/shared/vanilla/horizon-client/26.3']
    if mode == 'wasm-pulley': args += ['--wasm-interpreter']
    if mode == 'wasm-fuel': args += ['--wasm-fuel']
    args += ['--dedicated', '--nogui']
    metadata = dict(mode=mode, machines=count, time_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), host=os.uname().nodename,
        head=os.environ['BENCH_BINARY_HEAD'],
        sample_head=os.environ['BENCH_SAMPLE_HEAD'],
        production_binary_head=os.environ.get('BENCH_BINARY_HEAD'), guest_head=os.environ.get('BENCH_GUEST_HEAD'),
        binary_size=BIN.stat().st_size, binary_mtime=BIN.stat().st_mtime,
        profile=os.environ.get('BENCH_BUILD_PROFILE', 'debug smoke; not a performance result'), command=args,
        cpu_model=next(line.split(':', 1)[1].strip() for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),
        sample_source_note='same source per phase; dev runner native include, canonical wasm shared-group package; warmed before recording',
        loadavg=pathlib.Path('/proc/loadavg').read_text(), io_pressure=pathlib.Path('/proc/pressure/io').read_text(), world=str(home))
    (OUT/f'world-{mode}-{count}-method.json').write_text(json.dumps(metadata, indent=2))
    p = subprocess.Popen(args, cwd=home, env=env, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True)
    def send(s):
        if p.poll() is not None: raise RuntimeError(f'{mode} exited {p.returncode}: {home}')
        p.stdin.write(s+'\n'); p.stdin.flush()
    try:
        time.sleep(25)
        send('forceload add 0 0 31 31')
        # Give the server a tick to initialize block-entity container tables
        # before feeding; console input may otherwise drain in one tick.
        time.sleep(3)
        for i in range(count):
            x,z = i%32, i//32
            send(f'setblock {x} 65 {z} machine_sample:machine')
            send(f'item replace block {x} 65 {z} container.0 with minecraft:cobblestone 64')
            send(f'data merge block {x} 65 {z} {{progress:39}}')
        # Wait for setup to clear the console queue, then record steady-state ticks.
        time.sleep(15)
        if os.environ.get('BENCH_PROFILE'):
            (home/'bench-profile.arm').write_text('one warmed block-entity tick\n')
        send('data get block 0 65 0')
        if count > 1:
            send(f'data get block {(count-1)%32} 65 {(count-1)//32}')
        seconds = 60 if mode == 'wasm-pulley' and count == 1000 else 20
        send(f'horizon-report {seconds}')
        deadline = time.monotonic() + seconds + 120
        while not any(home.glob('reports/perf-*.json')):
            if p.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(f'{mode} {count}: report did not complete: {home}')
            time.sleep(1)
        send('data get block 0 65 0')
        if count > 1:
            send(f'data get block {(count-1)%32} 65 {(count-1)//32}')
        send('stop')
        try:
            p.wait(timeout=15)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()
        for f in home.glob('reports/perf-*'):
            shutil.copy(f, OUT / f'world-{mode}-{count}-{f.name}')
    finally:
        if p.poll() is None: p.kill(); p.wait()
        log.close()
        shutil.copy(home/'benchmark.log', OUT/f'world-{mode}-{count}.log')
        if os.environ.get('BENCH_PROFILE'):
            shutil.copy(home/'bench-profile.json', OUT/f'world-{mode}-{count}-profile.json')
        if (home/'guest-profile.json').exists():
            shutil.copy(home/'guest-profile.json', OUT/'guest-profile.json')
    output = (OUT/f'world-{mode}-{count}.log').read_text()
    assert output.count('Replaced 1 slot(s)') == count, f'{mode}: feed failed; see {home}'
    endpoints = [line for line in output.splitlines() if 'has the following block data' in line]
    assert len(endpoints) >= (4 if count > 1 else 2), f'{mode}: missing endpoints; see {home}'
    assert all('id: "minecraft:stone"' in line for line in endpoints), f'{mode}: no stone; see {home}'
    assert not any(s in output.lower() for s in ['wasm mod machine_sample refused', 'wasm trap']), f'{mode}: wasm refusal; see {home}'
    report = json.loads(next(OUT.glob(f'world-{mode}-{count}-perf-*.json')).read_text())
    assert sum(level['block_entities'] for level in report['levels']) == count
    assert report['players'] == 0 and all(level['mobs'] == 0 for level in report['levels'])
    print(mode,count,home, flush=True)

if __name__ == '__main__':
    for mode in ['native', 'wasm']:
        run(mode, 1)
