import asyncio
import trinity  # type: ignore

def call_me_back(room, caps):
    print(f"Called me back! I love you! {caps.rest}")
    for i, s in enumerate(caps):
        print(f"  [{i}] = {s}")
    try:
        response1 = trinity.make_text_response("what did you say?")
        response2 = trinity.make_text_response("what did you say?!!")
    except Exception as e:
        print("raising " + str(e))
        import pdb; pdb.set_trace()
        raise

    return [response1, response2]

async def secondary(room, caps):
    print(f"SECONDARY {caps.rest}")
    await trinity.send_text(room, f"SECONDARY {caps.rest}");
    await asyncio.sleep(3)
    await trinity.send_text(room, "waited 3 sec")
    await asyncio.sleep(3)
    await trinity.send_text(room, "...continuing")
    return "yeehaw"

def init():
    print("Initializating knowledge plugin")
    print("trinity = " + str(trinity))
    # print("trinity.app = " + str(trinity.app))
    trinity.register_input_handler(r"gwoink (\w+) (?P<rest>.*)", call_me_back)
    trinity.register_input_handler(r"async (?P<rest>.*)", secondary)
