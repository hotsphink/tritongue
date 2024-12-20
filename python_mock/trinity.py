def register_input_handler(*args):
    pass

class TextResponse(str):
    '''Dummy class, not corresponding to anything in the Rust trinity.'''
    pass

def make_text_response(text):
    return TextResponse(text)

async def send_text(room, text):
    print(text)

async def send_html(room, html, _text):
    print(html)

async def react(room, event_id, text):
    print(f"reaction to {event_id}: {text}")
