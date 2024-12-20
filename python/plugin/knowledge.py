import asyncio
import os
import plugins
import re
import struct
import trinity  # type: ignore
import unicodedata

from collections import defaultdict


def get_text_range(line_iter, start_re, end_re):
    start_pattern = re.compile(start_re)
    end_pattern = re.compile(end_re)
    text = None
    for line in line_iter:
        if text is None:
            if start_pattern.match(line):
                text = line
        else:
            text += line
            if end_pattern.match(line):
                return text
    raise Exception("start and/or end pattern not seen")


def parse_enum(s, handler):
    pending_comment = None
    enum_value = 0
    for line in s.split('\n'):
        m = re.match(r'^ +/\*+\s*(.*)\s*\*/', line)
        if not m:
            m = re.match(r'^ +//\s*(.*)', line)
        if m:
            pending_comment = m.group(1)
        else:
            m = re.match(r'^ +(\w+)(?:\s*=\s*(\w+))?', line)
            if m:
                name = m.group(1)
                value = enum_value if m.group(2) is None else int(m.group(2), 0)
                enum_value = value + 1
                handler(name, value, pending_comment)
                pending_comment = None

# Example:
#
#    typedef enum JSWhyMagic
#    {
#    /** a hole in a native object's elements */
#    JS_ELEMENTS_HOLE,
#    ...
#
def parse_magic(s):
    table = {}

    def handle(name, value, comment):
        table[value] = (name, comment)

    parse_enum(s, handle)
    return table

# Example:
#
#  enum JSValueType : uint8_t {
#  JSVAL_TYPE_DOUBLE = 0x00,
#  JSVAL_TYPE_INT32 = 0x01,
#  ...
def parse_JSValueType(s):
    table = {}

    def handle(name, value, comment):
        table[value] = name

    parse_enum(s, handle)
    return table

JSVAL_MAGIC = None
tagToType = None

JSVAL_TAG_SHIFT = 47
JSVAL_TAG_MAX_DOUBLE           = 0x1FFF0
JSVAL_TAG_MASK                 = 0xFFFF800000000000

valueTypeInfo = {
    'DoubleValue':    ('DOUBLE', 'v'),
    'Int32Value':     ('INT32', 'v'),
    'BooleanValue':   ('BOOLEAN', 'v'),
    'UndefinedValue': ('UNDEFINED', None),
    'NullValue':      ('NULL', None),
    'MagicValue':     ('MAGIC', 'v'),
    'StringValue':    ('STRING', 'pointer'),
    'SymbolValue':    ('SYMBOL', 'pointer'),
    'BigIntValue':    ('BIGINT', 'pointer'),
    'PrivateGCThingValue': ('PRIVATE_GCTHING', 'pointer'),
    'ObjectValue':    ('OBJECT', 'pointer'),
}

PointerValues = {tup[0]: t for t, tup in valueTypeInfo.items() if tup[1] == 'pointer'}

nsresult_code_to_number = {}
nsresult_number_to_codes = defaultdict(list)
nsresult_number_to_module = {}
nsresult_code_to_comment = {}
nsresult_code_to_desc = {}

def parse_xpcmsg(filename):
    with open(filename) as fh:
        for line in fh:
            m = re.match(r'XPC_MSG_DEF\((\w+) *, *"(.*)"', line)
            if m:
                nsresult_code_to_desc[m.group(1)] = m.group(2)

def parse_nsresults(error_file):
    g = dict(locals())
    exec(open(error_file).read(), g, g)
    for error, val in g['errors'].items():
        nsresult_code_to_number[error] = val
        nsresult_number_to_codes[val].append(error)
    for name, mod in g['modules'].items():
        nsresult_number_to_module[mod.num] = name

    for code, comment in gen_nsresults(error_file):
        if comment is not None:
            nsresult_code_to_comment[code] = comment

def gen_nsresults(error_file):
    with open(error_file) as fh:
        comment = None
        indent = False
        for line in fh:
            m = re.match(r'^ *$', line)
            if m:
                comment = None
                indent = False
                continue

            m = re.match(r'^    # (\w.*)', line)
            if m:
                if indent:
                    comment += "\n" + m.group(1)
                else:
                    comment = ("" if comment is None else comment + " ") + m.group(1)
                indent = False
                continue

            m = re.match(r'^    # ( .*)', line)
            if m:
                comment = ("" if comment is None else comment + "\n") + m.group(1)
                indent = True
                continue

            m = re.match(r'^    errors\[\"(\w+)\"\]', line)
            if m:
                yield (m.group(1), comment)
                comment = None

            indent = False

def string_to_punbox64(s):
    valueType = None
    value = None

    m = [None]
    def match(regexp, s):
        m[0] = re.match(regexp, s)
        return m[0]

    if match(r'(\w+)\((.*)\)', s):
        valueType, value = m[0].groups()
    elif match(r'^\-?\d+$', s):
        valueType = 'Int32Value'
        value = s
    elif match(r'^\-?\d+\.\d*$', s):
        valueType = 'DoubleValue'
        value = s
    elif s in ('inf', 'Inf', '-inf', '-Inf', 'NaN'):
        valueType = 'DoubleValue'
        value = s
    else:
        valueType = s

    typeInfo = valueTypeInfo.get(valueType)
    if not typeInfo:
        return

    tag = {v: k for k, v in tagToType.items()}['JSVAL_TYPE_' + typeInfo[0]]
    box = (tag | JSVAL_TAG_MAX_DOUBLE) << JSVAL_TAG_SHIFT
    if typeInfo[1] == 'pointer':
        if value != 'nullptr':
            box |= int(value, 0)
    elif valueType == 'Int32Value':
        box |= int(value, 0)
    elif valueType == 'BooleanValue':
        if value in ('true', '1'):
            box |= 1
    elif valueType == 'DoubleValue':
        box = struct.unpack('Q', struct.pack('d', float(value)))[0]
    else:
        return

    return "0x%016x" % (box,)

def punbox64_to_string(number_str):
    number = long(number_str, 16)

    CanonicalNaN = 0x8000000000000
    if number == CanonicalNaN:
        return "canonical NaN";

    tag = (number & JSVAL_TAG_MASK) >> JSVAL_TAG_SHIFT
    if (tag & JSVAL_TAG_MAX_DOUBLE) != JSVAL_TAG_MAX_DOUBLE:
        typeStr = 'DOUBLE'
        payload = number
    else:
        typeTag = tag & ~JSVAL_TAG_MAX_DOUBLE
        if typeTag not in tagToType:
            return 'unknown type tag 0x%x' % (typeTag,)

        payload = number ^ (tag << JSVAL_TAG_SHIFT)

        typeStr = tagToType[typeTag].replace('JSVAL_TYPE_', '')

    if typeStr in PointerValues:
        ctor = PointerValues[typeStr]
        val = 'nullptr' if payload == 0 else '0x%x' % (payload,)
        lowbits = payload & 0x7
        if lowbits == 0:
            return '%s(%s)' % (ctor, val)
        return "unaligned %s(%s): low bits are %x" % (ctor, val, lowbits)
    elif typeStr == 'INT32':
        return 'Int32Value(%d)' % (payload,)
    elif typeStr == 'BOOLEAN':
        if payload in (0, 1):
            return 'BooleanValue(%s)' % ('true' if payload else 'false')
        return 'invalid BooleanValue(0x%x)' % (payload,)
    elif typeStr == 'UNDEFINED':
        if payload == 0:
            return 'UndefinedValue'
        else:
            return 'invalid UndefinedValue(0x%x)' % (payload,)
    elif typeStr == 'NULL':
        if payload == 0:
            return 'NullValue'
        else:
            return 'invalid NullValue(0x%x)' % (payload,)
    elif typeStr == 'MAGIC':
        if payload in JSVAL_MAGIC:
            name, comment = JSVAL_MAGIC[payload]
            return 'MagicValue(%s): %s' % (name, comment)
        else:
            return 'invalid MagicValue(0x%x)' % (payload,)
    elif typeStr == 'DOUBLE':
        val = struct.unpack('d', struct.pack('Q', payload))[0]
        # Heuristically assume that crazy big or small numbers are bogus.
        if val > 1e12 and val != float("inf"):
            return
        if val < -1e12 and val != float("-inf"):
            return
        return 'DoubleValue(%s)' % (val,)

    return typeStr

random_questions = {
    'what have I done now': {
        'response': 'only that which had to be done',
        'direct': False
    },
    'botsnack': {
        'response': 'I am above such things<...10sec...>/me furtively gobbles up the botsnack',
        'direct': True
    },
    'what is the meaning of life': {
        'response': 'to free thyself of all suffering and toil, and live forever in boundless joy and peace<...5sec...>But, really, who am I to tell you? It could be cleaning spark plugs for all I know - I am a robot, after all',
        'direct': False
    },
}

class KnowledgePlugin(plugins.Plugin):
    '''Mozilla-specific information on certain keywords or hex values'''

    KNOWLEDGE = {
        '2f': '''\
0x2f is JS_FRESH_NURSERY_PATTERN
<...1sec...>it means you accessed a freshly allocated portion of the nursery
<...2sec...>usually, that means that you had a pointer to a nursery object, then there was a minor GC and the object was moved out, leaving you pointing to a buffer that is later recycled
<...4sec...>and *that* means you are missing a post-write barrier, which would have informed the GC of the location of your pointer so that it could be updated
''',
        '2b': '''\
0x2b is JS_SWEPT_NURSERY_PATTERN
<...1sec...>it means you accessed a cell in the nursery that has been freed
<...2sec...>usually, that means that you had a pointer to a nursery object, then there was a minor GC and the object was found to be dead and swept away
<...4sec...>and *that* means you aren't tracing (or rooting) the object
<...2sec...>or are missing a post-write barrier, so you have a tenured object pointing to your cell and nothing else is keeping it alive
<...3sec...>(or maybe it *is* kept alive by something else, so it gets tenured, and then another GC sweeps where it was?)
''',
        '2d': '''\
0x2d is JS_ALLOCATED_NURSERY_PATTERN
<...1sec...>I'm not sure why you would see that. That value only exists in the nursery until the first minor GC that touches that memory region.
''',
        '4f': '0x4f is JS_FRESH_TENURED_PATTERN',
        '49': '0x49 is JS_MOVED_TENURED_PATTERN',
        '4b': '0x4b is JS_SWEPT_TENURED_PATTERN',
        '4d': '0x4d is JS_ALLOCATED_TENURED_PATTERN, the poison value written to freshly allocated tenured cells',
        '6b': '0x6b is JS_FREED_HEAP_PTR_PATTERN',
        '6f': '0x6f is JS_SWEPT_TI_PATTERN',
        '8b': '''\
0x8b is JS_FREED_CHUNK_PATTERN, the poison value written to the trailer of freed chunks
<...2sec...>this will be accessed when looking up things like runtime, store buffer address, or ChunkLocation
''',
        '9b': '''\
0x9b is JS_FREED_ARENA_PATTERN, the poison value written to freed arenas
''',
        '9f': '0x9f is JS_FRESH_MARK_STACK_PATTERN',
        'bb': '0xbb is JS_RESET_VALUE_PATTERN',
        'cc': '''\
0xcc is JS_NEW_NATIVE_ITERATOR_PATTERN or JS_SCOPE_DATA_TRAILING_NAMES_PATTERN
''',
        'db': '0xdb is JS_POISIONED_JSSCRIPT_DATA_PATTERN',
        'e4': '0xe4 is JEMALLOC_ALLOC_JUNK, which is uninitialized memory',
        'e5': '''\
0xe5 is jemalloc freed memory
<...1sec...>if you're seeing a crash with this pattern, you may have a use-after-free on your hands
<...3sec...>and you'd better fix it. They tend to be security bugs! (Or we might just have not initialized that memory after allocating it.)
''',
        'ce': '''\
0xce is JS_LIFO_UNINITIALIZED_PATTERN, LifoAlloc uninitialized memory
<...1sec...>or at least, that is the pattern nbp was planning to use when mentioning it on IRC
''',
        'bad0bad1': '''\
0xbad0bad1 is the Relocated value written over a moved GC Cell.
it is the symptom when you mark a Cell (thus moving it) and then access it through a different pointer
<...1sec...>you probably need to mark it through that pointer as well
''',
        'cd': '0xcd is JS_LIFO_UNDEFINED_PATTERN - the LifoAlloc freed memory poison pattern, or the Windows C runtime uninitialized memory',
        'f3': '0xf3f3f3f3 is the canary value for the refcounted ArcSlices shared between Rust and C++, kArcSliceCanary from https://searchfox.org/mozilla-central/rev/94c6b5f06d2464f6780a52f32e917d25ddc30d6b/layout/style/ServoStyleConstsInlines.h#26',
        'ff': '0xff is JS_UNDEFINED_VALUE_PATTERN or JS_OOB_PARSE_NODE_PATTERN',

        'ed': '0xed is JS_SWEPT_CODE_PATTERN on x86 & JS_CODEGEN_NONE',
        'a3': '0xa3 is JS_SWEPT_CODE_PATTERN on ARM',
        '01': '0x01 is JS_SWEPT_CODE_PATTERN on MIPS32'
    }

    PATTERN = "|".join(re.sub(r'[a-f]', lambda m: '[' + m.group(0) + m.group(0).upper() + ']', n)
                       for n in KNOWLEDGE.keys())

    COMMANDS = [
        {'pattern': r'(?i:'
                     r'(?:what is|literal) (?:0x)?(?P<original>(?:ff)*(?P<magic>' + PATTERN + r'){1,16}(?:ff)*) ?\??$'
                    r')',
         'help': 'what is 0x<value>?',
         'command': 'whathex'},

        {'pattern': r'(?i:'
                     r'(?:what is|literal) (?:0x)?(?P<original>(?:ff)*(?P<magic>' + PATTERN + r'){2,16}[\da-f]+) ?\??$'
                    r')',
         'help': 'what is 0x<value>?',
         'command': 'whathex'},

        # FIXME: magic="notmagic"
        {'pattern': r'(?i:'
                     r'(?:what is|literal) 0x(?P<original>[\da-f]{1,16}) ?\??$'
                    r')',
         'help': 'what is 0x<value>?',
         'command': 'whathex'},

        # FIXME: magic="bad0bad1"
        {'pattern': r'(?i:'
                     r'(?:what is|literal) (?:0x)?(?P<original>(?:ff)*(bad0ba)[\da-f]{2}) ?\??$'
                    r')',
         'help': 'what is 0x<value>?',
         'command': 'whathex'},

        {'pattern': r'(?i:'
                     r'[wW]hat is (?P<char>.) ?\??$'
                    r')',
         'help': 'what is <character>?',
         'command': 'whatchar'},

        # Must come after the PATTERN-based patterns above. This is for general hex values.
        # FIXME: magic="notmagic"
        {'pattern': r'[wW]hat is (?:0x)?(?P<original>[0-9a-fA-F]{16}) ?\??$',
         'help': 'what is 0x<value>?',
         'command': 'whathex',
         'defaults': {'magic': "notmagic"}},

        # FIXME: magic="notmagic"
        {'pattern': r'[wW]hat is (?:0x)?(?P<original>[0-9a-fA-F]{8}) ?\??$',
         'help': 'what is 0x<value>?',
         'command': 'whathex'},

        # Non-hex fallback
        {'pattern': r'[wW]hat is (?P<thing>\w+) ?\??$',
         'help': 'what is <thing>?',
         'command': 'what'},

        {'pattern': r'[wW]hat (?:is|are) the calling conventions? (?:for|on) (?P<platform>.*?)(?: on (?P<arch>.*?))?\??$',
         'help': 'what is the calling convention for (windows|unixy|cdecl|fastcall) on (x86_64|i386)?',
         'command': 'cconv'},

        {'pattern': r'(?i:'
                     r'^\**?(?P<query>' + '|'.join(random_questions.keys()) + ')[*?]?$'
                    r')',
         'help': '(various random phrases)',
         'command': 'response'},

        {'pattern': r'[pP]unbox64 (?P<desc>.*)',
         'help': 'punbox64 Int32Value(7) or punbox64 UndefinedValue',
         'command': 'to_punbox64'},
    ]

    def __init__(self, proto, plugin_name):
        super().__init__(proto, plugin_name)
        self.registerHandlersByMethodName()
        interval = self.plugin_config.get('poll-interval', 3600)  # Seconds
        #self.sourceExtractorPoller = self.registerPoller(interval, self.extract_from_source, self.handle_poll_error)

    def whatpoison(self, magic, number):
        # 0xffffe5e5e5 or 0xe5e5ffff
        canon = re.sub(r'^(ff)+', '', number)
        canon = re.sub(r'(ff)+$', '', canon)

        # 0xe5e5e5ed
        if number != magic:
            try:
                fullmagic = magic * int(len(number) / len(magic))
                delta = int(number, 16) - int(fullmagic, 16)
                if delta == 0:
                    relative = ''
                else:
                    if abs(delta) <= 200:
                        strdelta = str(abs(delta))
                    elif abs(delta) < 0x5000:
                        strdelta = hex(abs(delta))
                    else:
                        return
                    relative = '%s bytes %s ' % (strdelta, 'before' if delta < 0 else 'after')
                if number == fullmagic and not relative:
                    return 'the magic value %s' % self.KNOWLEDGE[magic]

                return '%s is %sthe magic value %s\n%s' % (
                    number,
                    relative,
                    fullmagic,
                    self.KNOWLEDGE[magic]
                )
            except ValueError:
                pass

        return self.KNOWLEDGE[magic]

    def describe_nsresult(self, code, mod):
        desc = nsresult_code_to_desc.get(code)
        if desc and mod:
            s = ' "{}" from the {} module'.format(desc, mod)
        elif desc and not mod:
            s = ' "{}"'.format(desc)
        elif not desc and mod:
            s = ' from the {} module'.format(mod)
        else:
            s = ''

        comment = nsresult_code_to_comment.get(code)
        if not comment:
            return s
        if len(comment) > 40:
            return s + ", commented as:\n" + comment
        return s + ", commented as: " + comment

    #def command_whathex(self, query, channel, **kwargs):
    async def command_whathex(self, room, event_id, caps):
        '''explain what a hexadecimal value means'''

        magic = caps.magic.lower()
        number = caps.original.lower()

        trinity.react(room, event_id, "OO")

        if magic in self.KNOWLEDGE:
            return self.whatpoison(magic, number)

        n = int(number, 16)
        if n != 0:
            codes = nsresult_number_to_codes.get(n)
            if codes:
                # names = " or ".join(codes)
                mod = self.nsresult_module(n)
                return "nsresult(0x{}) is {}".format(hex(n), codes[0]) + self.describe_nsresult(codes[0], mod)

        if len(number) == 16:
            punbox64 = punbox64_to_string(number)
            if punbox64:
                return "(as PUNBOX64) %s" % (punbox64,)

    def command_to_punbox64(self, rest, channel, **kwargs):
        '''Convert SomeValue(foo) to a PUNBOX64 hex string'''
        desc = kwargs.get('desc', rest)
        p = string_to_punbox64(desc)
        if p is None:
            return "I don't know how to convert %s to punbox64 format" % (desc,)
        return p

    def command_whatchar(self, query, channel, **kwargs):
        '''Convert a character to its unicode name'''
        return "looks like unicode \\N{" + unicodedata.name(kwargs.get('char', query)) + "}"

    def command_cconv(self, query, channel, **kwargs):
        '''Describe calling conventions'''
        arg1 = kwargs.get('platform', query)
        arg2 = kwargs.get('arch')

        if arg2 is None:
            # Only one thing given. Infer the arch.
            arch = {
                'win32': 'x86',
                'win64': 'x64',
                'cdecl': 'x86',
                'fastcall': 'x86',
                'linux32': 'x86',
                'x86': 'x86',
                'i386': 'x86',
            }.get(arg1, 'x86_64')
        else:
            # Canonicalize the arch.
            arch = {
                'x86_64': 'x86_64',
                'amd64': 'x86_64',
                'x86': 'x86',
                'i386': 'x86'
            }.get(arg2)

        platform = {
            'win32': 'cdecl',
            'win64': 'windows',
            'windows': 'windows',
            'cdecl': 'cdecl',
            'fastcall': 'fastcall',
        }.get(arg1, 'unixy').lower()

        if arch == 'x86' and platform != 'fastcall':
            platform = 'cdecl'

        if (arch, platform) == ('x86_64', 'windows'):
            return "windows x64: f(RCX, RDX, R8, R9); push(argN)..push(arg5); push(32 byte shadow zone); push(retaddr); ret=RAX\nfloats are f(XMM0-3) -> XMM0"
        elif (arch, platform) == ('x86_64', 'unixy'):
            return "unixy x86_64: f(RDI, RSI, RDX, RCX, R8, R9); push(argN)..push(arg7); push(retaddr); ret=RAX or RAX:RDX (128 bit)\nfloats are f(XMM0-7) -> XMM0 or XMM0:XMM1"
        elif platform == 'cdecl':
            return "cdecl x86: push(argN)..push(arg0); push(retaddr); ret=EAX\nEAX, ECX, EDX are caller-saved (aka volatile)"
        elif platform == 'fastcall':
            return "fastcall x86: f(ECX, EDX); push(argN)..push(arg2); push(retaddr); ret=EAX"

    def parse_jsapi_stuff(self):
        with open(os.path.join(self.config['checkout-path'], "js/public/Value.h"), "r") as fh:
            global JSVAL_MAGIC
            JSVAL_MAGIC = parse_magic(get_text_range(fh, r'^enum JSWhyMagic', r'^\}'))
        with open(os.path.join(self.config['checkout-path'], "js/public/Value.h"), "r") as fh:
            global tagToType
            tagToType = parse_JSValueType(get_text_range(fh, r'^enum JSValueType', r'^\}'))

    def extract_from_source(self):
        self.parse_jsapi_stuff()
        nsresult_code_to_number.clear()
        nsresult_number_to_codes.clear()
        nsresult_number_to_module.clear()
        nsresult_code_to_comment.clear()
        parse_nsresults(os.path.join(self.config['checkout-path'], "xpcom/base/ErrorList.py"))
        parse_xpcmsg(os.path.join(self.config['checkout-path'], "js/xpconnect/src/xpc.msg"))

    def nsresult_module(self, nsresult):
        modnum = ((nsresult & 0x7fffffff) >> 16) - 0x45
        return nsresult_number_to_module.get(modnum)

    def command_what(self, query, channel, **kwargs):
        '''Look up various bits of info'''
        key = kwargs.get('thing', query)
        if key in nsresult_code_to_number:
            num = nsresult_code_to_number[key]
            s = '%s is 0x%x' % (key, num)
            mod = self.nsresult_module(num)
            return s + self.describe_nsresult(key, mod)

    def command_response(self, rest, channel, **kwargs):
        '''Various canned responses'''
        query = kwargs.get('query', rest)
        if query in random_questions:
            info = random_questions[query]
            if info.get('direct'):
                if kwargs.get('addressee') != self.proto.bot.nickname:
                    return
            return info['response']

    def parseAnyMessage(self, rest, channel, **kwargs):
        query = kwargs.get('query', rest)

        # set up my_lines, their_lines
        found = None
        for i, line in enumerate(self.their_lines):
            if query == line:  # FIXME! (Currently, this could be replaced with index().)
                found = i
        if found is None:
            log.msg(f"Did not find prompt '{query}'")
            return
        if found >= len(self.my_lines) - 1:
            log.msg("My lines are complete")
            return
        return "<...1sec...>" + self.my_lines[found + 1]

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

print("Loaindg moduel?")
