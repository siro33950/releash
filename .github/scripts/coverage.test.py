import os
from pathlib import Path
import select
import shlex
import subprocess
import tempfile
import unittest


class CoverageTest(unittest.TestCase):
    def test_profiles_survive_exit_and_sigkill_with_line_markers(self):
        sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
        version = subprocess.check_output(["rustc", "-vV"], text=True)
        host = next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))
        profdata = Path(sysroot) / "lib/rustlib" / host / "bin/llvm-profdata"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "probe.rs"
            source.write_text('#[test] fn profile_probe() { println!("ready"); let _ = std::io::stdin().read_line(&mut String::new()); }')
            binary = root / "probe"
            subprocess.run(["rustc", "--test", "-C", "instrument-coverage", *shlex.split(os.environ["RUSTFLAGS"]), str(source), "-o", str(binary)], check=True)
            profiles = root / "profiles"
            profiles.mkdir()
            for count, killed in enumerate([False, True], start=1):
                with self.subTest(killed=killed):
                    environment = {**os.environ, "LLVM_PROFILE_FILE": str(profiles / os.environ["LLVM_PROFILE_FILE_NAME"])}
                    with subprocess.Popen([str(binary), "--nocapture"], env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0) as child:
                        try:
                            while True:
                                self.assertTrue(select.select([child.stdout], [], [], 10)[0], "standalone readiness marker is missing")
                                line = child.stdout.readline()
                                self.assertTrue(line, "profile probe exited before readiness")
                                if line == b"ready\n":
                                    break
                            if killed:
                                child.kill()
                            else:
                                child.stdin.write(b"\n")
                                child.stdin.flush()
                            self.assertEqual(child.wait(timeout=10), -9 if killed else 0)
                        finally:
                            if child.poll() is None:
                                child.kill()
                                child.wait()
                    raw = list(profiles.glob("*.profraw"))
                    self.assertEqual(len(raw), 1, "processes must share an initialized profile")
                    merged = profiles / "merged.profdata"
                    subprocess.run([str(profdata), "merge", "-sparse", *map(str, raw), "-o", str(merged)], check=True)
                    counts = subprocess.check_output([str(profdata), "show", "--all-functions", "--counts", str(merged)], text=True)
                    self.assertIn(f"Function count: {count}", counts)


if __name__ == "__main__":
    unittest.main()
