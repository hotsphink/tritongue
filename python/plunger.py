#!/usr/bin/python


#################### NOT BEING USED ######################

import asyncio
import sys
import re
import importlib
import inspect
import json
import os
import requests
import traceback
import urllib

from collections import defaultdict

VERSION = "1.0"


def include(filename):
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), filename)
    exec(open(path).read(), data)

data = { 'include': include }

include("config.py")
config = data['config']

import plugins

def uniq(collection):
    allowed = set(collection)
    return [item for item in collection if item in allowed and allowed.remove(item) is None]


class Bot(object):
    def __init__(self, nick, config, dummy=False):
        self.state = defaultdict(dict)
        self.config = config


class CustomIRCBotProtocol(irc.IRCClient):
    token = None

    def __init__(self, bot, **kwargs):
        self.bot = bot
        self.commandHandlers = {}
        self.eventHandlers = {}
        self.config = config
        self.plugins = {}
        self.wantVerbose = False
        self.state = defaultdict(dict)
        print(config)
        for name, info in config.get('plugins', {}).items():
            if name.startswith('xxx-'):
                log.msg("(skipping plugin %s)" % name)
                continue
            log.msg("Loading plugin " + name + " for " + self.nickname)
            exec('from plugins.{name} import {klass}'.format(name=name, klass=info['class']))
            self.plugins[name] = eval('%s(self, "%s")' % (info['class'], name))

    def isVerbose(self):
        '''Spew long outputs to channel instead of using pastebin'''
        return self.isBatch() or self.wantVerbose

    def setVerbose(self, verbose=True):
        self.verbose = verbose

    def signedOn(self):
        plugins.startup(self)

    def command_source(self, rest, channel, **kwargs):
        return 'source code is at %s\npatches welcome' % self.config.get('source-url', 'https://hg.sr.ht/~sfink/mrgiggles')

    def handleEvent(self, name, event):
        handlers = self.eventHandlers.get(name, [])
        deferreds = []
        for handler in handlers:
            try:
                d = handler['handler'](event)
                if d:
                    deferreds.append(d)
            except Exception as e:
                log.err("Exception while handling %s event: %s" % (name, str(e)))
                log.err(traceback.format_exc())

        # TODO: Actually do something.


    async def handle_message(self, channel, message):
        func, rest, kwargs = None, None, {}
        fallbacks = []
        full_message = message

        if message.startswith('!'): # trigger command
            command, _sep, rest = message.lstrip('!').partition(' ')
            command = str(command)
            # Get the function corresponding to the command given.
            func = self.resolve_command(command)
            if not func:
                self.debug("Unknown command '{command}' in channel {channel}".format(command=command, channel=channel))
        else:
            for name, plugin in self.plugins.items():
                try:
                    parts = plugin.parseMessage(message, channel)
                    if parts:
                        command, rest, kwargs = parts
                        command = str(command)
                        if command == 'parseAnyMessage':
                            fallbacks.append(plugin.parseAnyMessage)
                            message = full_message  # FIXME!
                        else:
                            func = self.resolve_command(command)
                        break
                except Exception:
                    yuk = traceback.format_exception(*sys.exc_info()[:])
                    if 'maintainer' in self.config:
                        self.msg(self.config['maintainer'], "Plugin %s threw an exception: %s" % (name, "".join(yuk)))
                    else:
                        log.err(yuk)

            m = [None]
            def matches(r,s):
                m[0] = r.search(s)
                return m[0]

            if func:
                log.msg("func=%s rest=%s" % (func, rest))

        # FIXME! This only allows a single arbitrary message handler.
        if func is None and fallbacks:
            func = fallbacks[0]
            rest = full_message  # FIXME

        if not func:
            return

        rest = str(rest)

        try:
            result = func(rest, channel, **kwargs)
            if inspect.isawaitable(result):
                result = await result
        except Exception as e:
            self._show_error(e)

        # Whatever is returned is sent back as a reply:
        if channel == self.bot.nickname:
            # When channel == self.bot.nickname, the message was sent to the bot
            # directly and not to a channel. So we will answer directly too:
            await self._send_message(result, nick)
        else:
            # Otherwise, send the answer to the channel, and use the nick
            # as addressing in the message itself:
            await self._send_message(channel, nick)

    async def _send_message(self, msg, target, nick=None):
        if msg is None:
            return

        if not isinstance(msg, list):
            msg = [ msg ]

        for line in msg:
            # Split into chunks of text separated by delays (or None
            # for no delay).
            pieces = self.delay_re.split(line)
            while pieces:
                text = pieces.pop(0)
                if len(text) > 0:
                    if text.startswith("/me "):
                        self.describe(target, text.replace("/me ", ""))
                    else:
                        if nick:
                            text = '%s: %s' % (nick, text)
                        await self.msg(target, text)

                if pieces:
                    text = pieces.pop(0)
                    delay = float(text) if text else 1.0
                    await asyncio.sleep(delay)

    def _show_error(self, failure):
        failure.printTraceback()
        return failure.getErrorMessage()

    def command_ping(self, rest, channel, **kwargs):
        return 'pong'

    def get_commands(self):
        prefixes = ["command_", "private_command_"]
        cmds = {p: [] for p in prefixes}
        for attr in dir(self):
            for prefix in prefixes:
                if attr.startswith(prefix):
                    cmds[prefix].append(attr[len(prefix):])
        return (cmds[p] for p in prefixes)

    def private_command_echo(self, rest, channel, source=None, **kwargs):
        return rest

    def help_message_for(self, topic, source, kind=None):
        is_auth = self.is_authorized(source)
        msg = []
        if kind in ('any', 'plugin') and topic in self.plugins:
            verbosity = 2 if kind == 'plugin' else 1
            msg.append(self.plugins[topic].help(verbosity=verbosity, is_auth=is_auth))

        if kind in ('any', 'command'):
            verbosity = 2 if kind == 'command' else 1
            for name, plugin in self.plugins.items():
                cmd_msg = plugin.help(command=topic, verbosity=verbosity, is_auth=is_auth)
                if cmd_msg:
                    if msg:
                        msg.append(unichr(0x3000))
                    msg.append(cmd_msg)
                    break

        if msg:
            return msg

        return ["I have no help for '%s'" % topic]

    def help_message(self, source):
        source_url = self.config.get('source-url', 'https://hg.sr.ht/~sfink/mrgiggles')
        msg = ['help for %s (%s):' % (self.nickname, source_url)]
        cmds, privcmds = self.get_commands()
        is_auth = self.is_authorized(source)
        if is_auth:
            cmds.extend(["*" + c for c in privcmds])
        msg.append("builtin commands: " + ", ".join(cmds))
        msg.append("use help <plugin> for detailed help on a particular plugin, or help <command> for a single command")
        for name, plugin in self.plugins.items():
            msg.append(plugin.help(verbosity=0, is_auth=is_auth))
        msg.append("Documentation is also available at https://wiki.mozilla.org/Mrgiggles")
        return msg

    def command_help(self, rest, channel, source=None, **kwargs):
        parts = rest.split(' ', 1)
        if len(parts) == 1:
            topic = rest
            kind = 'any'
        else:
            kind, topic = parts

        if topic:
            return "\n".join(self.help_message_for(topic, source, kind=kind))
        else:
            return "\n".join(self.help_message(source))

    def private_command_reload(self, rest, channel, source=None, **kwargs):
        if rest == 'config':
            module = reload(sys.modules['mrgiggles_config'])
            self.config.clear()
            self.config.update(module.config)
            return "reloaded configuration (most changes will not take effect immediately)"

        if rest not in self.plugins:
            return "unknown plugin '%s'" % (rest,)
        sendback = channel if channel.startswith("#") else source
        self.msg(sendback, "reloading %s" % (rest,))
        try:
            oldplugin = self.plugins[rest]
            module = importlib.reload(sys.modules[oldplugin.__module__])
            plugin = getattr(module, oldplugin.__class__.__name__)(self, rest)
            self.plugins[rest] = plugin
            oldplugin.unload()
            plugin.registerHandlers()
            plugin.registerEventHandlers()
            plugin.startup()
            return "reloaded %s" % (plugin.__module__)
        except Exception:
            return traceback.format_exc()

    def command_auth(self, rest, channel, source=None, **kwargs):
        if rest == config['password']:
            self.userInfo.setdefault(source, {})['authorized'] = True
            self.userInfo.sync()
            self.msg(source, "password accepted")
        else:
            self.msg(source, "incorrect password")
        return None

    def command_unauth(self, rest, channel, source=None, **kwargs):
        try:
            del self.userInfo[source]['authorized']
            self.userInfo.sync()
            self.msg(source, "no longer authorized")
        except:
            self.msg(source, "you were never authorized in the first place")
        return None

    def pastebin(self, lines, style='text'):
        body = '\n'.join(lines)
        maxlen = 20000
        if style == 'text' and len(body) > maxlen:
            body = body[0:maxlen] + "...(truncated)"
        filename = 't.txt' if format == 'text' else 'doc.html'
        payload = {
            'description': 'pastebin-like gist for %s' % config['nick'],
            'public': True,
            'files': {
                filename: {
                    'content': body
                },
            },
        }
        auth = {'Authorization': 'bearer ' + config['gist_api_token']}
        gist_url = "%s/%s" % (config['gist_api_url'], config['gists']['pastebin_' + style])
        r = requests.patch(gist_url, headers=auth, data=json.dumps(payload))
        result_url = r.json()['files'][filename]['raw_url']
        if style == 'html':
            result_url = result_url.replace(config['gist_usercontent_url'], config['rawgit_url'])

        if 'bitly_token' in self.config:
            bitlyUrl = "https://api-ssl.bitly.com/v3/shorten?access_token=%s&longUrl=%s" % (self.config['bitly_token'], urllib.quote(result_url))
            r = requests.get(bitlyUrl)
            result_url = r.json()['data']['url']
        return result_url

    def multilines(self, lines, shortprefix='', longprefix='', prefix=None, maxinline=3, style='text'):
        if prefix is not None:
            shortprefix = shortprefix or prefix
            longprefix = longprefix or prefix
        if style != 'text':
            return normal(longprefix + self.pastebin(lines, style=style))
        elif len(lines) <= maxinline or self.isVerbose():
            return normal(shortprefix + '\n'.join(lines))
        else:
            return normal(longprefix + self.pastebin(lines))

    def complain(self, failure):
        log.err(failure)
        return failure

    def handle_spawned_failure(self, err, channel):
        log.err(err)
        return "The command I ran for that? It failed. Blame sfink. And tell him to check my logs."

    async def command_shorten(self, url, channel, **kwargs):
        """Create a bit.ly shortened url. Ugh, this is not picked up by help."""
        key = (kwargs['source'], channel)
        if url in ('', 'that', 'that url'):
            url = self.bot.previous_url(channel)
            log.msg("previous url was %r" % (url,))
            if not url:
                return
        history = self.bot.state['long_urls'].setdefault(key, [url])
        # Wait 2 seconds before assuming the URL is complete, to
        # handle URLs split over multiple lines.
        await asyncio.sleep(2)
        full_url = ''.join(history)
        log.msg("Full url = %r" % (full_url,))
        del self.bot.state['long_urls'][key]
        return await self.shortLink(full_url)

    async def shortLink(self, url):
        if 'bitly_token' not in self.config:
            return url

        bitlyUrl = "https://api-ssl.bitly.com/v3/shorten?access_token=%s&longUrl=%s" % (self.config['bitly_token'], urllib.quote(url))
        bitlyPage = await getPage(bitlyUrl)
        result = json.loads(bitlyPage)
        if result['status_code'] != 200:
            raise(Exception(result['status_txt']))
        return result['data']['url']

def init():
    mainbot = Bot(config)

    # Since we're running in the foreground anyway, show what's happening by
    # logging to stdout.
    log.startLogging(sys.stdout)
