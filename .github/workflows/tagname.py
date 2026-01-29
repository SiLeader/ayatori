import argparse
import re

version_regex = re.compile(r"^v(\d+\.\d+\.\d+)$")

parser = argparse.ArgumentParser()
parser.add_argument("--tag", required=True)
parser.add_argument("--commit", required=True)
parser.add_argument("--base", required=True)

args = parser.parse_args()

tags = []

if version_regex.match(args.tag):
    tags.append(f"{args.base}:{args.tag}")
    tags.append(f"{args.base}:latest")
else:
    commit = args.commit[:8]
    tags.append(f"{args.base}:{commit}")

print(','.join(tags))
