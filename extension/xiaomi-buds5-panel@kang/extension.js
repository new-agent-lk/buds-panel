'use strict';

const {Clutter, Gio, GLib, GObject, St} = imports.gi;
const Main = imports.ui.main;
const QuickSettings = imports.ui.quickSettings;
const PopupMenu = imports.ui.popupMenu;
const {Slider} = imports.ui.slider;

const BUS_NAME = 'io.github.kang.XiaomiBuds5';
const OBJECT_PATH = '/io/github/kang/XiaomiBuds5';
const IFACE = 'io.github.kang.XiaomiBuds5';
const MODES = ['关闭', '降噪', '通透'];
const LEVELS = ['舒适', '均衡', '深度'];
const CODECS = {
    sbc: 'SBC', sbc_xq: 'SBC-XQ', aac: 'AAC', aptx: 'aptX Classic', aptx_hd: 'aptX HD',
    aptx_adaptive_hq: 'aptX Adaptive HQ（接近无损）',
    aptx_adaptive: 'aptX Adaptive（动态）',
    aptx_adaptive_ll: 'aptX Adaptive LL（低延迟）',
    ldac_hq: 'LDAC HQ', ldac_sq: 'LDAC SQ', ldac_mq: 'LDAC MQ',
};

const IFACE_XML = `
<node>
  <interface name="io.github.kang.XiaomiBuds5">
    <method name="Refresh"/>
    <method name="SetNoiseMode"><arg name="mode" type="u" direction="in"/></method>
    <method name="SetNoiseLevel"><arg name="mode" type="u" direction="in"/><arg name="level" type="u" direction="in"/></method>
    <method name="Connect"/>
    <method name="SetAudioBackend"><arg name="backend" type="s" direction="in"/><arg name="codec" type="s" direction="in"/></method>
    <method name="ListCodecs"><arg name="codecs" type="a(ss)" direction="out"/></method>
    <property name="Connected" type="b" access="read"/>
    <property name="Name" type="s" access="read"/>
    <property name="Address" type="s" access="read"/>
    <property name="Battery" type="a{sv}" access="read"/>
    <property name="NoiseMode" type="u" access="read"/>
    <property name="NoiseLevel" type="u" access="read"/>
    <property name="LastError" type="s" access="read"/>
    <signal name="StateChanged"><arg name="state" type="a{sv}"/></signal>
  </interface>
</node>`;

function unpack(v) {
    return v == null ? null : v.deepUnpack();
}

function formatBattery(battery) {
    if (!battery)
        return '电量未知';
    const values = battery instanceof GLib.Variant ? unpack(battery) : battery;
    const names = {left: '左', right: '右', case: '盒'};
    return ['left', 'right', 'case'].map(key => {
        const raw = values[key];
        if (!raw)
            return `${names[key]}=?`;
        const pair = raw instanceof GLib.Variant ? unpack(raw) : raw;
        const [pct, charging] = pair;
        return pct < 0 ? `${names[key]}=?` : `${names[key]}=${pct}%${charging ? '⚡' : ''}`;
    }).join('  ');
}

class ControlPanel extends St.BoxLayout {
    constructor() {
        super({vertical: true, style_class: 'buds-panel', reactive: true});
        this._volumeValue = 0;
        this._volumeLock = false;
        this._build();
    }

    _build() {
        const head = new St.BoxLayout({style_class: 'buds-header'});
        this._title = new St.Label({text: 'Xiaomi Buds 5', style_class: 'buds-title'});
        head.add_child(this._title);
        const close = new St.Button({style_class: 'buds-close', child: new St.Icon({icon_name: 'window-close-symbolic', icon_size: 16})});
        close.connect('clicked', () => this.onClose?.());
        head.add_child(close);
        this.add_child(head);

        this._status = new St.Label({text: '正在连接…', style_class: 'buds-muted'});
        this.add_child(this._status);
        this._battery = new St.Label({text: '电量：读取中…', style_class: 'buds-muted'});
        this.add_child(this._battery);
        this._codec = new St.Label({text: '协议：读取中…', style_class: 'buds-muted'});
        this.add_child(this._codec);

        this.add_child(new St.Label({text: '音量', style_class: 'buds-section'}));
        const volumeRow = new St.BoxLayout({style_class: 'buds-row'});
        volumeRow.add_child(new St.Icon({icon_name: 'audio-volume-low-symbolic', icon_size: 16}));
        this._volume = new Slider(0);
        this._volume.x_expand = true;
        this._volume.accessible_name = 'Xiaomi Buds 5 音量';
        this._volume.connect('notify::value', () => this._volumeChanged());
        volumeRow.add_child(this._volume);
        this._volumeLabel = new St.Label({text: '0%', style_class: 'buds-value'});
        volumeRow.add_child(this._volumeLabel);
        this.add_child(volumeRow);

        this.add_child(new St.Label({text: '降噪模式', style_class: 'buds-section'}));
        const modes = new St.BoxLayout({style_class: 'buds-buttons'});
        this._modeButtons = [];
        MODES.forEach((name, mode) => {
            const b = new St.Button({label: name, style_class: 'buds-button', x_expand: true});
            b.connect('clicked', () => this.onMode?.(mode));
            modes.add_child(b);
            this._modeButtons.push(b);
        });
        this.add_child(modes);

        this._levelSection = new St.Label({text: '降噪强度', style_class: 'buds-section'});
        this.add_child(this._levelSection);
        const levels = new St.BoxLayout({style_class: 'buds-buttons'});
        this._levelButtons = [];
        LEVELS.forEach((name, level) => {
            const b = new St.Button({label: name, style_class: 'buds-button', x_expand: true});
            b.connect('clicked', () => this.onLevel?.(level));
            levels.add_child(b);
            this._levelButtons.push(b);
        });
        this._levelRow = levels;
        this.add_child(levels);

        this.add_child(new St.Label({text: '音频后端', style_class: 'buds-section'}));
        this._backendBox = new St.BoxLayout({style_class: 'buds-buttons'});
        this._backendButtons = {};
        for (const [id, label] of [
            ['pulse', '兼容模式'],
            ['adaptive', '高质量模式'],
        ]) {
            const b = new St.Button({label, style_class: 'buds-button', x_expand: true});
            b.connect('clicked', () => this.onBackend?.(id));
            this._backendBox.add_child(b);
            this._backendButtons[id] = b;
        }
        this.add_child(this._backendBox);

        this.add_child(new St.Label({text: '蓝牙协议', style_class: 'buds-section'}));
        this._codecBox = new St.BoxLayout({vertical: true, style_class: 'buds-codecs'});
        this.add_child(this._codecBox);
    }

    _volumeChanged() {
        if (this._volumeLock)
            return;
        const percent = Math.round(this._volume.value * 100);
        this._volumeValue = percent;
        this._volumeLabel.text = `${percent}%`;
        this.onVolume?.(percent);
    }

    setVolume(percent) {
        this._volumeValue = percent;
        this._volumeLock = true;
        this._volume.value = Math.max(0, Math.min(1, percent / 100));
        this._volumeLock = false;
        this._volumeLabel.text = `${percent}%`;
    }

    setCodecs(codecs, current) {
        this._codecBox.destroy_all_children();
        this._codec = this._codec || new St.Label();
        for (const item of codecs) {
            const name = item.name || item;
            const label = item.description || CODECS[name] || name;
            const b = new St.Button({
                label: name === current ? `${label}  ✓` : label,
                style_class: name === current ? 'buds-codec selected' : 'buds-codec',
                x_expand: true,
            });
            b.connect('clicked', () => this.onCodec?.(name));
            this._codecBox.add_child(b);
        }
    }

    update(state, currentCodec, codecs) {
        const connected = unpack(state.connected ?? state.Connected) || false;
        const name = unpack(state.name ?? state.Name) || 'Xiaomi Buds 5';
        this._title.text = name;
        if (!connected) {
            this._status.text = '未连接';
            this._battery.text = '电量：—';
            this._codec.text = '协议：—';
            return;
        }
        this._status.text = '已连接';
        this._battery.text = `电量：${formatBattery(state.battery ?? state.Battery)}`;
        const mode = unpack(state.noiseMode ?? state.NoiseMode) || 0;
        const level = unpack(state.noiseLevel ?? state.NoiseLevel) || 0;
        const backend = unpack(state.audioBackend ?? state.AudioBackend) || 'pulse';
        const codec = currentCodec || '';
        this._codec.text = `协议：${CODECS[codec] || codec || '—'}`
            + (codec.startsWith('aptx_adaptive') ? ' · 48 kHz/24-bit' : '');
        for (const [id, button] of Object.entries(this._backendButtons))
            button.set_style_class_name(id === backend ? 'buds-button selected' : 'buds-button');
        this._modeButtons.forEach((b, i) => b.set_style_class_name(i === mode ? 'buds-button selected' : 'buds-button'));
        this._levelSection.visible = mode === 1;
        this._levelRow.visible = mode === 1;
        this._levelButtons.forEach((b, i) => b.set_style_class_name(i === level ? 'buds-button selected' : 'buds-button'));
        this.setCodecs(codecs || [], currentCodec || '');
    }
}

const BudsToggle = GObject.registerClass(
class BudsToggle extends QuickSettings.QuickMenuToggle {
    _init() {
        super._init({label: 'Xiaomi Buds 5', iconName: 'audio-headphones-symbolic', toggleMode: false});
        this.menu.setHeader('audio-headphones-symbolic', 'Xiaomi Buds 5', '正在连接…');
        this._panel = new ControlPanel();
        this._panel.onClose = () => this.menu.close();
        this._panel.onVolume = value => this._setVolume(value);
        this._panel.onMode = mode => this._call('SetNoiseMode', new GLib.Variant('(u)', [mode]));
        this._panel.onLevel = level => this._call('SetNoiseLevel', new GLib.Variant('(uu)', [1, level]));
        this._panel.onCodec = codec => this._setCodec(codec);
        this._panel.onBackend = backend => this._setBackend(backend);
        this._item = new PopupMenu.PopupBaseMenuItem({activate: false, reactive: true});
        this._item.add_child(this._panel);
        this.menu.addMenuItem(this._item);

        this._proxy = null;
        this._signalId = 0;
        this._volumePollId = GLib.timeout_add_seconds(2, () => {
            this._readVolume(value => this._panel.setVolume(value));
            return GLib.SOURCE_CONTINUE;
        });
        this._connectProxy();
        this.connect('clicked', () => this.menu.open());
    }

    _connectProxy() {
        this._proxy = Gio.DBusProxy.new_for_bus_sync(
            Gio.BusType.SESSION, Gio.DBusProxyFlags.DO_NOT_AUTO_START,
            Gio.DBusInterfaceInfo.new_for_xml(IFACE_XML), BUS_NAME, OBJECT_PATH, IFACE, null);
        this._signalId = this._proxy.connectSignal('StateChanged', (_p, _s, args) => this._update(args[0]));
        this._proxy.init_async(GLib.PRIORITY_DEFAULT, null, (proxy, res) => {
            try {
                proxy.init_finish(res);
                this._refreshProperties();
            } catch (e) {
                this._panel._status.text = '控制服务未启动';
            }
        });
    }

    _call(method, params = null) {
        if (!this._proxy)
            return;
        const timeout = method === 'SetAudioBackend' ? 75000 : 5000;
        this._proxy.call(method, params, Gio.DBusCallFlags.NONE, timeout, null, (proxy, res) => {
            try {
                proxy.call_finish(res);
                this._refreshProperties();
            } catch (e) {
                this._panel._status.text = e.message;
            }
        });
    }

    _refreshProperties() {
        if (!this._proxy)
            return;
        try {
            const state = {};
            for (const key of this._proxy.get_cached_property_names())
                state[key] = this._proxy.get_cached_property(key);
            this._update(state);
        } catch (e) {
            this._panel._status.text = e.message;
        }
    }

    _update(state) {
        this._readVolume(value => this._panel.setVolume(value));
        const codec = unpack(state.codec ?? state.Codec) || '';
        const codecs = unpack(state.codecs ?? state.Codecs) || [];
        this._panel.update(state, codec, codecs.map(item => ({name: item[0], description: item[1]})));
        const connected = unpack(state.connected ?? state.Connected) || false;
        const mode = unpack(state.noiseMode ?? state.NoiseMode) || 0;
        const name = unpack(state.name ?? state.Name) || 'Xiaomi Buds 5';
        this.label = name;
        this.subtitle = connected ? `降噪：${MODES[mode] || '未知'}` : '未连接';
        this.iconName = connected ? (mode === 1 ? 'audio-headphones-symbolic' : mode === 2 ? 'audio-volume-high-symbolic' : 'audio-volume-muted-symbolic') : 'audio-headphones-symbolic';
    }

    _readVolume(callback) {
        const proc = Gio.Subprocess.new(['pactl', 'get-sink-volume', '@DEFAULT_SINK@'], Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_SILENCE);
        proc.communicate_utf8_async(null, null, (p, res) => {
            try {
                const [, out] = p.communicate_utf8_finish(res);
                const m = out.match(/(\d+)%/);
                if (m) callback(Number(m[1]));
            } catch (_) {}
        });
    }

    _setVolume(value) {
        Gio.Subprocess.new(['pactl', 'set-sink-volume', '@DEFAULT_SINK@', `${value}%`], Gio.SubprocessFlags.NONE);
    }

    _setCodec(codec) {
        const backend = unpack(this._proxy?.get_cached_property('AudioBackend')) || 'pulse';
        this._call('SetAudioBackend', new GLib.Variant('(ss)', [backend, codec]));
    }

    _setBackend(backend) {
        const codec = backend === 'adaptive' ? 'aptx_adaptive_hq' : 'aptx';
        this._panel._status.text = '正在切换音频后端，可能需要 5–15 秒…';
        this._call('SetAudioBackend', new GLib.Variant('(ss)', [backend, codec]));
    }

    destroy() {
        if (this._proxy && this._signalId)
            this._proxy.disconnectSignal(this._signalId);
        if (this._volumePollId) {
            GLib.source_remove(this._volumePollId);
            this._volumePollId = 0;
        }
        super.destroy();
    }
});

const BudsIndicator = GObject.registerClass(
class BudsIndicator extends QuickSettings.SystemIndicator {
    _init() {
        super._init();
        this._indicator = this._addIndicator();
        this._indicator.icon_name = 'audio-headphones-symbolic';
        this.quickSettingsItems.push(new BudsToggle());
        Main.panel.statusArea.quickSettings._indicators.insert_child_at_index(this, 0);
        Main.panel.statusArea.quickSettings._addItems(this.quickSettingsItems);
    }

    destroy() {
        this.quickSettingsItems.forEach(item => item.destroy());
        this._indicator.destroy();
        super.destroy();
    }
});

let indicator = null;
function init() {}
function enable() { indicator = new BudsIndicator(); }
function disable() {
    if (indicator) {
        indicator.destroy();
        indicator = null;
    }
}
