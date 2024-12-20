import asyncio
import logging
import os
import re
import trinity  # type: ignore

logger = logging.getLogger(__name__)

# Plugin interface
#
# Plugins, upon creation, register a set of regexes that (normally) every
# incoming message will be matched against.
#
# In addition, plugins can register a callback to be invoked on every
# otherwise-unhandled message. (The regex matches will happen first.)
#
# There should probably be a mechanism to "capture" followup messages, so that
# if a beginning token is matched, then the plugin will be fed every message up
# to an end token or condition.
#
# I am moving away from magic fields or methods that get noticed and/or invoked
# implicitly by the plugin manager, and instead relying on explicit
# registration during startup.
#
# Hopefully, this will make things more testable as well?
#
# Remaining magic:
#
# The plugin parent class provides facilities for auto-registering methods
# that follow a naming convention.


def log(message):
    logger.info(message)


class MatchGroup(int):
    '''For specifying that an argument should be filled in with a re.match's group'''
    pass


class Plugin(object):
    def __init__(self, proto, plugin_name):
        self.proto = proto
        self.userInfo = proto.userInfo
        self.config = proto.config
        self.plugin_config = proto.config.get('plugins', {}).get(plugin_name, {})
        self.name = plugin_name
        #self.registerHandlers()
        #self.registerEventHandlers()
        self.registerCommands()

    def registerCommands(self):
        for spec in self.COMMANDS:
            for pat, _private in [("command_{cmd}", False),
                                  ("private_command_{cmd}", True)]:
                name = pat.format(cmd=spec['command'])
                if func := getattr(self, name):
                    async def wrapped_func(room, event_id, caps, func=func):
                        #print(f"called wrapped_func(self={self}, room={room}, caps={list(caps)}), func={func}")
                        result = await func(room, event_id, caps)
                        await self.proto.process_message(room, result)

                    trinity.register_input_handler(spec['pattern'], wrapped_func, spec.get('defaults'))
                    break


    def registerHandlersByMethodName(self, target=None):
        '''Scan `self` and register command handlers for any method name
        starting with "command_" or "command_private_". Set the command
        handlers on the `target` object.
        '''
        target = target or self.proto
        for method in dir(self):
            parts = method.split("command_", 1)
            if len(parts) == 2:
                maybeprivate, name = parts
                if maybeprivate == '':
                    target.registerCommand(name, getattr(self, method), authRequired=False)
                elif maybeprivate == 'private_':
                    target.registerCommand(name, getattr(self, method), authRequired=True)

    def registerEventHandlers(self, target=None):
        target = target or self.proto
        for method in dir(self):
            parts = method.split("event_", 1)
            if len(parts) == 2:
                # sfink doesn't know what he is doing. descriptors!
                # binding! classes! attributes!
                bound = getattr(self.__class__, method).__get__(self, self.__class__)
                target.eventHandlers.setdefault(parts[1], []).append({ 'owner': self, 'handler': bound })

    def startup(self):
        pass

    def shutdown(self):
        pass

    def unload(self):
        # Do any shutdown processing needed
        try:
            self.shutdown()
        except:
            self.debug("shutdown failed for %r" % (self,))
            pass

        # Each handler in commandHandlers records the plugin instance
        # that it belongs to. Rip out all of the commands for this plugin.
        for command, handler in self.proto.commandHandlers.items():
            if handler['handler'] == self:
                handler.clear()
                del self.proto.commandHandlers[command]

        # Similarly for eventHandlers, except an event is associated
        # with a list of handlers.
        for command, handlers in self.proto.eventHandlers.items():
            handlers[:] = filter(lambda handler: handler['owner'] != self, handlers)

    def registerPoller(self, interval, func, handle_err):
        print(f"ignoring plugin.registerPoller({interval}, {func},)")

    def varFilename(self, name):
        var_template = self.proto.config.get('var-path', 'var/{nick}')
        var_path = var_template.format(**self.proto.config)
        return os.path.join(var_path, name)

    @staticmethod
    def argValue(value, m, kwargs):
        if isinstance(value, MatchGroup):
            return m.group(value)
        elif isinstance(value, dict):
            return {key: Plugin.argValue(val, m, kwargs) for key, val in value.items()}
        elif isinstance(value, list):
            return [Plugin.argValue(elt, m, kwargs) for elt in value]
        elif isinstance(value, tuple):
            return [Plugin.argValue(elt, m, kwargs) for elt in value]
        elif hasattr(value, '__call__'):
            return value(**kwargs)
        else:
            return value

    def parseMessage(self, message, channel, source, addressee=None):
        kwargs = {'message': message,
                  'channel': channel,
                  'source': source,
                  'addressee': addressee}

        for spec in getattr(self, 'COMMANDS', []):
            if spec.get('trace'):
                import pdb; pdb.set_trace()
            if spec.get('direct-only') and not addressee:
                continue
            m = spec['pattern'].match(message)
            if not m:
                continue
            if 'filter' in spec:
                if not spec['filter'](m):
                    continue

            rv = [
                spec['command'],
                Plugin.argValue(spec.get('args', []), m, kwargs),
                Plugin.argValue(spec.get('kwargs', {}), m, kwargs),
            ]

            log("parseMessage (%s) = %r" % (message, rv))

            return rv

        if hasattr(self, 'parseAnyMessage'):
            log(f"making an attempt at parseAnyMessage")
            return [
                'parseAnyMessage',
                '',
                {},
            ]

    def runCommand(self, name, rest, channel, **kwargs):
        func = getattr(self, 'command_' + name, getattr(self, 'private_command_' + name, None))
        log("plugin command: %r" % (func,))
        return func(rest, channel, **kwargs)

    def snark(self, **kwargs):
        return self.proto.snark(**kwargs)

    def msg(self, *args):
        return self.proto.msg(*args)

    def fail(self, *args):
        return self.proto.fail(*args)

    def complain(self, failure):
        return self.proto.complain(failure)

    def debug(self, *args):
        return self.proto.debug(*args)

    def multilines(self, *args, **kwargs):
        return self.proto.multilines(*args, **kwargs)

    # With command:
    #   if command not known, return None
    #   if not verbose, return the code docs for command
    #   if verbose, return the code docs + any COMMAND docs
    # Without command:
    #   if not verbose, return the list of commands supported by the plugin
    #   if verbose, return all code docs + any COMMAND docs
    def help(self, verbosity=0, is_auth=False, command=None):
        log("help({what}) called with verbosity={verbosity} command={command}".format(what=self.__class__, verbosity=verbosity, command=command))

        commands = []
        docs = {}
        for attr in dir(self.__class__):
            if attr.startswith("command_"):
                cmd = attr.replace("command_", "")
            elif attr.startswith("private_command_") and is_auth:
                cmd = attr.replace("private_command_", "*")
            else:
                continue
            commands.append(cmd)
            if getattr(self, attr).__doc__:
                docs[cmd] = getattr(self, attr).__doc__
            else:
                docs[cmd] = 'no help available'

        # Was a specific command or plugin requested?
        if command:
            # Unknown command?
            if command not in docs:
                return

            # Do we need to look at COMMANDS?
            if verbosity < 1 or not hasattr(self, 'COMMANDS'):
                return docs[command]

        elif verbosity < 1 or not hasattr(self, 'COMMANDS'):
            # No specific command requested and we don't want verbose:
            plugin_doc = self.__doc__ or "no description available"
            return "'{}' plugin - {}\n  commands: {}".format(self.name, plugin_doc, ", ".join(commands))

        # We have a COMMANDS field. Use it to get some more detailed help.

        msg = []
        patterns = {}
        for spec in self.COMMANDS:
            # Gather a list of all help lines for each command
            help_lines = patterns.setdefault(spec['command'], [])
            cmd_lines = spec.get('help', spec['pattern'].pattern)
            if isinstance(cmd_lines, list):
                help_lines.extend(cmd_lines)
            else:
                help_lines.append(cmd_lines)

        suppress_patterns = not command and (verbosity == 1 and len(self.COMMANDS) >= 15)
        want_patterns = (verbosity >= 2 or not suppress_patterns)

        for cmd in commands:
            if command and cmd != command:
                continue
            msg.append("command '%s': %s" % (cmd, docs[cmd]))
            if want_patterns:
                msg += ['  "%s"' % p for p in patterns.get(cmd, [])]

        if not command:
            msg = ["  " + t for t in msg]
            if self.__doc__:
                msg.insert(0, "commands implemented: " + " ".join(commands))
                msg.insert(0, "the %s plugin: %s" % (self.name, self.__doc__))
            else:
                msg.insert(0, "the %s plugin implements: " % self.name + " ".join(commands))

        if suppress_patterns:
            msg.append("use 'help command <command>' to see the recognized strings for a given command")

        return "\n".join(msg)


def startup(proto):
    for plugin in proto.plugins.values():
        log("Firing up %s" % (plugin.name,))
        plugin.startup()

class Bot(object):
    delay_re = re.compile(r'\<\.\.\.(?:([\d\.]+)\s?sec\.\.\.)?\>')

    def __init__(self):
        self.config = {'plugins': {'knowledge': { 'class': 'KnowledgePlugin'}}}
        self.plugins = {}
        self.userInfo = {}
        for name, info in self.config.get('plugins', {}).items():
            # import
            print(f'from plugin.{name} import {info["class"]}')
            exec(f'from plugin.{name} import {info["class"]}')
            # instantiate
            print(f'{info["class"]}(self, "{name}")')
            self.plugins[name] = eval(f'{info["class"]}(self, "{name}")')

    def registerCommand(self, name, callback, authRequired=False):
        print(f"ignoring registerCommand({name}, auth={authRequired})")

    def registerPoller(self, interval, func, handle_err):
        print(f"ignoring registerPoller({interval}, {func},)")

    async def process_message(self, target, msg, nick=None):
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
                    if nick:
                        text = '%s: %s' % (nick, text)
                    await trinity.send_text(target, text)

                if pieces:
                    text = pieces.pop(0)
                    delay = float(text) if text else 1.0
                    await asyncio.sleep(delay)

def init():
    bot = Bot()
