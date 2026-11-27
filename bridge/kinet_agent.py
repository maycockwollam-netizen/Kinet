#!/usr/bin/env python3
"""Kinet → Kinetic SDK bridge (Mốc 3).

Chạy một agent Kinetic SDK thật và in kết quả ra stdout để Kinet đọc lại.

Cách dùng:
    python3 kinet_agent.py "goal text"

Biến môi trường:
    PYTHONPATH        → thư mục chứa gói kinetic_sdk (bắt buộc)
    OPENAI_API_KEY    → nếu có, dùng LiteLLMClient (LLM thật)
                        nếu không, dùng MockLLMClient offline (đủ để demo)
    KINET_MODEL       → tên model khi dùng LLM thật (mặc định gpt-4o-mini)
"""

import os
import sys

from kinetic_sdk.agent import Agent
from kinetic_sdk.security import PermissivePolicy
from kinetic_sdk.tool.base import Tool, ToolResult


class EchoTool(Tool):
    """Một tool đơn giản để thấy vòng lặp tool-calling của SDK chạy thật."""

    name = "echo"
    description = "Echo a message back to the caller"
    parameters = {
        "type": "object",
        "properties": {"message": {"type": "string"}},
        "required": ["message"],
    }

    def execute(self, message: str) -> ToolResult:
        return ToolResult(output=f"[echo] {message}")


def build_agent() -> Agent:
    if os.environ.get("OPENAI_API_KEY"):
        from kinetic_sdk.llm.client import LiteLLMClient

        model = os.environ.get("KINET_MODEL", "gpt-4o-mini")
        llm = LiteLLMClient(model=model)
    else:
        from kinetic_sdk.testing import MockLLMClient, text_response

        # Offline: LLM giả trả về chuỗi chứa goal, để vòng lặp SDK chạy được.
        llm = MockLLMClient([text_response("goal complete")])
    return Agent(llm=llm, tools=[EchoTool()], permission_policy=PermissivePolicy())


def main() -> int:
    goal = sys.argv[1] if len(sys.argv) > 1 else "Say hello"
    try:
        result = build_agent().run(goal)
    except Exception as exc:  # noqa: BLE001 — báo về Kinet để hiển thị
        print(f"ERR_BRIDGE: {exc}", file=sys.stderr)
        return 2
    print(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())