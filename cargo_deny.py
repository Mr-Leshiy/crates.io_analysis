import asyncio
import logging

logger = logging.getLogger(__name__)


class CargoDenyInfo:
    def __init__(self, advisories: list[int]):
        self.advisories = advisories

    def __str__(self):
        return f"advisories: {self.advisories}"


class CargoDenyAdvisoryInfo:
    lints = {
        "vulnerability",
        "notice",
        "unmaintained",
        "unsound",
        "yanked",
        "index-failure",
        "index-cache-load-failure",
        "advisory-not-detected",
        "advisory-ignored",
        "unknown-advisory",
        "yanked-ignored",
        "yanked-not-detected",
    }

    async def analyze(crate_name, dir_name, config_path) -> list[int]:
        """
        Analyze a Rust project for advisory issues using multiple lint levels.

        Runs `cargo deny check advisories` asynchronously for each lint level
        defined in `CargoDenyAdvisoryInfo.lints`. Each check is configured to deny
        a single lint level while allowing the others, in order to isolate the
        number of issues specific to each.

        Args:
            dir_name (str): Path to the Rust project directory.
            config_path (str): Path to the `cargo deny` configuration file.

        Returns:
            list[int]: A list of integers representing the number of advisory
            issues found under each lint level. The order matches
            `CargoDenyAdvisoryInfo.lints`.
        """
        return await asyncio.gather(
            *[
                CargoDenyAdvisoryInfo.advisories_check(
                    crate_name, dir_name, config_path, entry
                )
                for entry in CargoDenyAdvisoryInfo.lints
            ]
        )

    async def advisories_check(
        crate_name,
        dir_name,
        config_path,
        lint,
        num_of_retries: int = 5,
        timeout: int = 120,
    ) -> int:
        for attempt in range(1, num_of_retries + 1):
            command = " ".join(
                [
                    "cargo",
                    "deny",
                    "check",
                    "advisories",
                    "--exclude-dev",
                    "--show-stats",
                    f"--config={config_path}",
                    f"--deny={lint}",
                    *[f"--allow={l}" for l in CargoDenyAdvisoryInfo.lints if l != lint],
                ]
            )
            proc = await asyncio.subprocess.create_subprocess_shell(
                command,
                cwd=f"{dir_name}",
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                stdin=asyncio.subprocess.DEVNULL,
            )
            try:
                status_code = asyncio.wait_for(await proc.wait(), timeout)
                if status_code != 0:
                    logger.error(
                        f"Cargo deny advisory {lint} for {crate_name} returns with {status_code} on attempt {attempt}. Terminating and retry..."
                    )
                    proc.terminate()
                    attempt += 1
                    continue

                out, _ = await proc.communicate()
                res = CargoDenyAdvisoryInfo.errors_amount(out)
                return res
            except asyncio.TimeoutError:
                logger.error(
                    f"Cargo deny advisory {lint} for {crate_name} timed out after {timeout} seconds on attempt {attempt}. Terminating and retry..."
                )
                proc.terminate()
                attempt += 1
                continue
        return None

    def is_ok(out: bytes) -> bool:
        return out.decode("utf-8").strip().split()[1] == "ok"

    def errors_amount(out: bytes) -> int:
        return int(out.decode("utf-8").strip().split()[2])
