"""Fixed synthetic inference responses for the two pinned provider clients."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
from urllib.parse import urlsplit
from uuid import uuid4


REPLY = "Synthetic fixture ready."


def anthropic_events():
    yield "message_start", {"message": {
        "id": f"msg_{uuid4().hex}", "type": "message", "role": "assistant",
        "model": "claude-sonnet-4-6", "content": [], "stop_reason": None,
        "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 0},
    }}
    yield "content_block_start", {"index": 0, "content_block": {"type": "text", "text": ""}}
    yield "content_block_delta", {"index": 0, "delta": {"type": "text_delta", "text": REPLY}}
    yield "content_block_stop", {"index": 0}
    yield "message_delta", {
        "delta": {"stop_reason": "end_turn", "stop_sequence": None},
        "usage": {"output_tokens": 3},
    }
    yield "message_stop", {}


def codex_events():
    response_id = f"resp_{uuid4().hex}"
    item = {
        "id": f"msg_{uuid4().hex}", "type": "message", "role": "assistant",
        "status": "completed", "content": [{"type": "output_text", "text": REPLY, "annotations": []}],
    }
    yield "response.created", {"response": {"id": response_id, "output": []}}
    yield "response.output_item.added", {
        "output_index": 0, "item": {**item, "status": "in_progress", "content": []},
    }
    yield "response.output_text.delta", {"output_index": 0, "content_index": 0, "delta": REPLY}
    yield "response.output_item.done", {"output_index": 0, "item": item}
    yield "response.completed", {"response": {
        "id": response_id, "output": [item],
        "usage": {"input_tokens": 10, "output_tokens": 3, "total_tokens": 13},
    }}


class Inference(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        routes = {"/v1/messages": anthropic_events, "/v1/responses": codex_events}
        route = routes.get(urlsplit(self.path).path)
        length = self.headers.get("Content-Length", "")
        if route is None:
            self.send_error(404)
            return
        if not length.isascii() or not length.isdecimal() or len(length) > 8 or int(length) > 2**21:
            self.send_error(413)
            return
        self.connection.settimeout(5)
        try:
            self.rfile.read(int(length))
            body = "".join(
                f"event: {event}\ndata: {json.dumps({'type': event, **payload})}\n\n"
                for event, payload in route()
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except OSError:
            return


def start():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Inference)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server
