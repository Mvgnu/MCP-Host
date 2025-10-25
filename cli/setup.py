from pathlib import Path
from setuptools import setup

root = Path(__file__).resolve().parents[1]
readme = (root / "README.md").read_text()

setup(
    name="mcp-host-cli",
    version="0.1.0",
    py_modules=["mcp_cli", "gen_python_sdk", "gen_ts_sdk", "get_config"],
    package_dir={"": "../scripts"},
    entry_points={"console_scripts": ["mcp-cli=mcp_cli:main"]},
    description="CLI utilities for MCP Host",
    long_description=readme,
    long_description_content_type="text/markdown",
    install_requires=["requests", "fastapi", "uvicorn"],
    python_requires=">=3.8",
)
