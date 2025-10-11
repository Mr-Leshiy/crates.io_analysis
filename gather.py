import csv
import asyncio
import aiohttp
import aiofiles
import tempfile
import logging
import argparse
import requests

from cargo_deny import CargoDenyInfo, CargoDenyAdvisoryInfo

CRATES_IO_URL = "https://crates.io/api"
USER_AGENT_HEADER = (
    "crates.io_analysis (https://github.com/Mr-Leshiy/crates.io_analysis)"
)
logger = logging.getLogger(__name__)


def endpoint_url(endpoint):
    return f"{CRATES_IO_URL}/{endpoint}"


class CrateInfo:
    def colum_names() -> list:
        return [
            "name",
            "version",
            "upload_time",
            "downloads",
            "recent_downloads",
            *[f"ad-{l}" for l in CargoDenyAdvisoryInfo.lints],
        ]

    def to_row(
        name: str,
        version: str,
        upload_time: str,
        downloads: int,
        recent_downloads: int,
        advisories: list[int],
    ) -> list:
        return [
            name,
            version,
            upload_time,
            downloads,
            recent_downloads,
            *advisories,
        ]


async def main(args):
    fname = "crates_info.csv"
    logger.info(f"Loading crates info into the {fname}")
    with open(fname, "w") as f:       
        writer = csv.writer(f)
        writer.writerow(CrateInfo.colum_names())

        processed_amount = 0
        next_page = args.next_page

        crates_per_page = 100
        while next_page != None:
            if next_page == "":
                next_page = (
                    f"?sort=new&include_yanked=no&per_page={crates_per_page}"
                )

            info = await crates_info(f"{next_page}")
            crates = await analyze_crates(info["crates"])
            processed_amount += len(crates)
            writer.writerows(crates)
            next_page = info["meta"]["next_page"]

            logger.info(
                f"processed {processed_amount}/{info['meta']['total']}, next_page: {next_page}"
            )
        logger.info(
            f"All crates info loaded, total amount: {info['meta']['total']}, processed amount: {processed_amount}"
        )


async def crates_info(args: str, num_of_retries: int = 5):
    logger.info(f"Trying to get crates info, args: {args}")

    for attempt in range(1, num_of_retries + 1):
        resp = requests.get(endpoint_url(f"v1/crates{args}"), headers={"User-Agent": USER_AGENT_HEADER})
        if resp.status_code == 200:
            return resp.json()
        else:
            logger.error(
                f"Request failed with status {resp.status} on attempt {attempt}. Response: {resp.text}. Retrying..."
            )
            await asyncio.sleep(5)  # Optional backoff between retries
    return None


async def analyze_crates(crates: list):
    async with aiohttp.ClientSession() as session:
        crates_iter = filter(lambda c: not c["yanked"], crates)
        crates_iter = map(
            lambda c: analyse_crate(session, c["name"], c["newest_version"]),
            crates_iter,
        )
        # filter out all `None` elements returned by 'analyse_crate'
        crates_iter = filter(lambda v: v != None, await asyncio.gather(*crates_iter))
        crates_iter = map(
            lambda v: CrateInfo.to_row(
                name=v[1]["name"],
                version=v[1]["newest_version"],
                upload_time=v[1]["updated_at"],
                downloads=v[1]["downloads"],
                recent_downloads=v[1]["recent_downloads"],
                advisories=v[0].advisories,
            ),
            zip(crates_iter, crates),
        )
        return list(crates_iter)


async def analyse_crate(
    session: aiohttp.ClientSession, name: str, version: str
) -> CargoDenyInfo:
    "Return 'None' if cannot analyse the crate for some reason"

    crate_name = f"{name}_{version}"
    fname = f"{crate_name}.tar.gz"
    with tempfile.TemporaryDirectory(dir="./") as tmpdirname:
        logger.info(f"Downloading crate {name}/{version}")
        async with (
            session.get(
                endpoint_url(f"v1/crates/{name}/{version}/download"),
                headers={"User-Agent": USER_AGENT_HEADER},
            ) as resp,
            aiofiles.open(f"{tmpdirname}/{fname}", "wb") as f,
        ):
            if resp.content_type != "application/gzip":
                return None

            chunk_size = 1024 * 4
            while True:
                data = await resp.content.read(chunk_size)
                if not data:
                    break
                await f.write(data)

        # unpack archive

        proc = await asyncio.subprocess.create_subprocess_exec(
            "tar",
            "-xf",
            f"{tmpdirname}/{fname}",
            "--strip-components=1",
            "-C",
            tmpdirname,
            stdout=asyncio.subprocess.DEVNULL,
            stderr=asyncio.subprocess.DEVNULL,
        )
        await proc.wait()
        logger.info(f"Analyzing crate {name}/{version}...")
        res = CargoDenyInfo(
            advisories=await CargoDenyAdvisoryInfo.analyze(tmpdirname, "../deny.toml")
        )
        logger.info(f"Crate {name}/{version} analyzed")
        return res


if __name__ == "__main__":
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(message)s",
        datefmt="%m/%d/%Y %I:%M:%S",
    )
    parser = argparse.ArgumentParser(description="crates.io gatheting tool.")
    parser.add_argument(
        "--next_page",
        type=str,
        default="",
        help="crates.io 'v1/crates' endpoint 'seek' query argument",
    )
    args = parser.parse_args()
    asyncio.run(main(args))
