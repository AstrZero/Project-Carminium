(function () {
  'use strict';

  if (window.__CARMINIUM_PLATFORM__ !== 'ios') return;

  var NAV_CATALOG = [
    { id: 'your_mix',  btnId: 'nav-your-mix',  icon: 'auto_awesome', labelKey: 'nav.yourMix',   fallback: '探新' },
    { id: 'music',     btnId: 'nav-music',     icon: 'music_note',   labelKey: 'nav.music',     fallback: '音乐' },
    { id: 'albums',    btnId: 'nav-albums',    icon: 'album',        labelKey: 'nav.albums',    fallback: '专辑' },
    { id: 'artists',   btnId: 'nav-artists',   icon: 'person',       labelKey: 'nav.artists',   fallback: '艺术家' },
    { id: 'folders',   btnId: 'nav-folders',   icon: 'folder_open',  labelKey: 'nav.folders',   fallback: '文件夹' },
    { id: 'history',   btnId: 'nav-history',   icon: 'history',      labelKey: 'nav.history',   fallback: '历史' },
    { id: 'liked',     btnId: 'nav-liked',     icon: 'favorite',     labelKey: 'nav.liked',     fallback: '喜爱' },
    { id: 'playlists', btnId: 'nav-playlists', icon: 'queue_music',  labelKey: 'nav.playlists', fallback: '歌单' },
    { id: 'settings',  btnId: 'nav-settings',  icon: 'settings',     labelKey: 'nav.settings',  fallback: '设置' },
  ];
  var DEFAULT_NAV_ITEMS = ['your_mix', 'music', 'albums', 'settings'];
  var MAX_NAV_ITEMS = 4;

  var _navItems = DEFAULT_NAV_ITEMS.slice();
  var _moreStyle = 'popup'; // 'popup' | 'page'
  var _morePopupEl = null;

  function _t(key, fallback) {
    if (window.App && App.i18n && App.i18n.t) {
      var val = App.i18n.t(key);
      if (val && val !== key) return val;
    }
    return fallback || key;
  }

  function _isPhone() {
    return window.innerWidth <= 767;
  }

  function _normalizeNavItems(list) {
    var out = [];
    var seen = {};
    var valid = {};
    NAV_CATALOG.forEach(function (c) { valid[c.id] = 1; });
    (Array.isArray(list) ? list : []).forEach(function (id) {
      if (!valid[id] || seen[id] || out.length >= MAX_NAV_ITEMS) return;
      seen[id] = 1;
      out.push(id);
    });
    return out.length ? out : DEFAULT_NAV_ITEMS.slice();
  }

  function _getHiddenNavItems() {
    var shown = {};
    _navItems.forEach(function (id) { shown[id] = 1; });
    return NAV_CATALOG.filter(function (c) { return !shown[c.id]; });
  }

  // ── 1. Settings Platform Filtering & iOS Nav Customization Hooks ─────

  var DESKTOP_ONLY_SECTION_IDS = { shortcuts: 1 };
  var DESKTOP_ONLY_GROUP_KEYS = {
    'settings.group.audioOutput': 1,
    'settings.group.taskbarLyrics': 1,
    'settings.group.discordRpc': 1,
    'settings.group.memoryOpt': 1,
    'settings.group.memoryInfo': 1,
    'settings.group.eq': 1,
    'settings.group.audioEffects': 1,
    'settings.group.spatialAudio': 1,
    'settings.group.vbe': 1,
    'settings.group.systemIntegration': 1,
  };
  var DESKTOP_ONLY_BINDS = {
    wasapi_exclusive: 1,
    wasapi_exclusive_notice: 1,
    audio_output_device: 1,
    automix: 1,
    radical_transitions: 1,
    video_background: 1,
    np_default_view: 1,
    smtc_lyrics: 1,
    taskbar_thumb_controls: 1,
    track_notify: 1,
    taskbar_lyrics: 1,
    discord_rpc: 1,
    tag_editor_path: 1,
    cache_dir: 1,
    memory_optimization: 1,
    _memory_info: 1,
    eq_enabled: 1,
    dynamic_bass: 1,
    compressor_enabled: 1,
    vocal_enhance: 1,
    guitar_friendly: 1,
    hearing_protection: 1,
    volume_normalization: 1,
    volume_normalization_target: 1,
    spatial_audio: 1,
    vbe_enabled: 1,
  };

  function _isDesktopOnlyRow(row) {
    if (!row || !row.bind) return false;
    if (DESKTOP_ONLY_BINDS[row.bind]) return true;
    if (row.bind.indexOf('taskbar_lyrics') === 0) return true;
    if (row.bind.indexOf('discord_rpc') === 0) return true;
    if (row.bind.indexOf('spatial_') === 0) return true;
    if (row.bind.indexOf('vbe_') === 0) return true;
    if (row.bind.indexOf('eq_') === 0) return true;
    return false;
  }

  window.__iosFilterSettingsSections = function (sections) {
    if (!Array.isArray(sections)) return sections;
    return sections
      .filter(function (sec) { return !DESKTOP_ONLY_SECTION_IDS[sec.id]; })
      .map(function (sec) {
        if (sec.isPage || !Array.isArray(sec.groups)) return sec;
        var groups = sec.groups
          .filter(function (g) { return !DESKTOP_ONLY_GROUP_KEYS[g.titleKey]; })
          .map(function (g) {
            var rows = (g.rows || []).filter(function (r) { return !_isDesktopOnlyRow(r); });
            return Object.assign({}, g, { rows: rows });
          })
          .filter(function (g) { return g.rows.length > 0; });

        if (sec.id === 'general') {
          groups.unshift({
            titleKey: 'ios.group.localImport',
            title: 'iOS 本地曲库',
            rows: [
              {
                type: 'action',
                bind: '_ios_import_files',
                icon: 'folder_open',
                label: '导入独立音乐库',
                sub: '从「文件」App 选择音频或文件夹导入到应用私有目录；也可直接把歌曲放到「我的 iPhone → Project Carminium」自动扫描',
                buttonText: '选择音频',
                buttonIcon: 'upload_file',
                onAction: function () {
                  App.utils.call('pick_audio_files').then(function (dir) {
                    if (dir && App.utils && App.utils.toast) {
                      App.utils.toast('正在扫描导入的音频…');
                    }
                  });
                },
              },
              {
                type: 'action',
                bind: '_ios_audio_diagnostics',
                icon: 'info',
                label: '系统音频与灵动岛诊断',
                sub: '查看 Bundle ID、签名描述文件、SceneDelegate、AVAudioSession 及 AirPods 事件实时日志',
                buttonText: '打开诊断',
                buttonIcon: 'info',
                onAction: function () {
                  _openIosDiagnosticsModal();
                },
              },
            ],
          });
        }

        if (sec.id === 'appearance') {
          groups.push({
            titleKey: 'ios.group.bottomNav',
            rows: [
              {
                type: 'action',
                bind: 'ios_nav_items',
                icon: 'dock_to_bottom',
                label: '自定义底部导航栏入口',
                sub: '挑选最多 4 个常用页面放到底部，其余自动收进「更多」',
                buttonText: '自定义',
                buttonIcon: 'edit',
                onAction: function () {
                  var host = document.getElementById('page-container');
                  _openIosNavEditor(host);
                },
              },
              {
                type: 'select',
                bind: 'ios_nav_more_style',
                icon: 'more_horiz',
                label: '「更多」展开方式',
                sub: '决定点击底栏「更多」时如何呈现其余页面',
                options: [
                  { value: 'popup', label: '浮层菜单（推荐）' },
                  { value: 'page',  label: '独立网格页' },
                ],
                onChange: function (val) {
                  _moreStyle = (val === 'page') ? 'page' : 'popup';
                },
              },
            ],
          });
        }

        return Object.assign({}, sec, { groups: groups });
      })
      .filter(function (sec) { return sec.isPage || (sec.groups && sec.groups.length > 0); });
  };

  function _refreshIosNavCard(container) {
    if (!container) return;
    var row = container.querySelector('.settings-row[data-bind="ios_nav_items"]');
    if (!row) return;
    var body = row.querySelector('.settings-row-body');
    if (!body) return;
    var chips = body.querySelector('#ios-nav-chips');
    if (!chips) {
      chips = document.createElement('div');
      chips.id = 'ios-nav-chips';
      chips.className = 'settings-np-secondary-chips';
      chips.style.marginTop = '8px';
      body.appendChild(chips);
    }
    chips.innerHTML = _navItems.map(function (id) {
      var item = NAV_CATALOG.find(function (c) { return c.id === id; });
      if (!item) return '';
      return '<span class="md-chip settings-np-chip">' +
        '<span class="material-symbols-rounded">' + item.icon + '</span>' +
        '<span class="settings-np-chip-label">' + _t(item.labelKey, item.fallback) + '</span>' +
      '</span>';
    }).join('');
  }

  function _openIosNavEditor(container) {
    var draft = _navItems.slice();
    var overlay = document.createElement('div');
    overlay.className = 'dialog-overlay open';
    overlay.style.cssText = 'position:fixed;inset:0;z-index:100020;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,0.48);';

    function renderModal() {
      var gridHtml = NAV_CATALOG.map(function (item) {
        var idx = draft.indexOf(item.id);
        var selected = idx >= 0;
        var disabled = !selected && draft.length >= MAX_NAV_ITEMS;
        return '' +
          '<button type="button" class="np-actions-editor-item' +
            (selected ? ' selected' : '') + (disabled ? ' disabled' : '') +
            '" data-nav-id="' + item.id + '"' + (disabled ? ' disabled' : '') + '>' +
            '<span class="material-symbols-rounded np-actions-editor-item-icon">' + item.icon + '</span>' +
            '<span class="settings-row-body"><span class="np-actions-editor-item-label">' +
              _t(item.labelKey, item.fallback) +
            '</span></span>' +
            '<span class="material-symbols-rounded np-actions-editor-check">' +
              (selected ? 'check_circle' : 'radio_button_unchecked') +
            '</span>' +
          '</button>';
      }).join('');

      overlay.innerHTML = '' +
        '<div class="md-dialog np-actions-editor" style="background:var(--md-surface-container-high);border-radius:28px;padding:20px;width:min(380px,calc(100vw - 24px));display:flex;flex-direction:column;gap:14px;">' +
          '<div style="font-size:18px;font-weight:600;color:var(--md-on-surface);">自定义底部导航栏</div>' +
          '<div class="np-actions-editor-body">' +
            '<p class="np-actions-editor-hint">选择 1~4 个常用页面常驻底部导航栏，其余页面自动收纳到「更多」中。</p>' +
            '<div class="np-actions-editor-grid md-seglist">' + gridHtml + '</div>' +
          '</div>' +
          '<div style="display:flex;align-items:center;gap:8px;justify-content:flex-end;">' +
            '<span class="np-actions-editor-count">已选 ' + draft.length + ' / ' + MAX_NAV_ITEMS + '</span>' +
            '<button type="button" class="btn-text" data-role="cancel">取消</button>' +
            '<button type="button" class="btn-filled" data-role="save">保存</button>' +
          '</div>' +
        '</div>';

      overlay.querySelectorAll('[data-nav-id]').forEach(function (btn) {
        btn.addEventListener('click', function () {
          var id = this.getAttribute('data-nav-id');
          var i = draft.indexOf(id);
          if (i >= 0) {
            if (draft.length > 1) draft.splice(i, 1);
          } else if (draft.length < MAX_NAV_ITEMS) {
            draft.push(id);
          }
          renderModal();
        });
      });

      var cancelBtn = overlay.querySelector('[data-role="cancel"]');
      var saveBtn = overlay.querySelector('[data-role="save"]');
      if (cancelBtn) cancelBtn.addEventListener('click', close);
      if (saveBtn) {
        saveBtn.addEventListener('click', function () {
          _navItems = _normalizeNavItems(draft);
          App.backend.save_settings(JSON.stringify({ ios_nav_items: _navItems }));
          _applyBottomNavLayout();
          _refreshIosNavCard(container);
          close();
        });
      }
    }

    function close() {
      if (overlay.parentNode) overlay.parentNode.removeChild(overlay);
    }

    overlay.addEventListener('click', function (e) {
      if (e.target === overlay) close();
    });

    renderModal();
    document.body.appendChild(overlay);
  }

  function _openIosDiagnosticsModal() {
    var overlay = document.createElement('div');
    overlay.className = 'dialog-overlay open';
    overlay.style.cssText = 'position:fixed;inset:0;z-index:100020;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,0.52);padding:16px;';

    overlay.innerHTML = '' +
      '<div class="md-dialog" style="background:var(--md-surface-container-high);border-radius:24px;padding:18px;width:min(460px,calc(100vw - 24px));max-height:82vh;display:flex;flex-direction:column;gap:12px;">' +
        '<div style="font-size:17px;font-weight:600;color:var(--md-on-surface);">iOS 系统音频与灵动岛诊断</div>' +
        '<textarea readonly data-role="diag-box" style="width:100%;height:52vh;min-height:240px;resize:none;border-radius:14px;border:1px solid var(--md-outline-variant);background:var(--md-surface-container-lowest);color:var(--md-on-surface);font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11.5px;line-height:1.45;padding:10px;box-sizing:border-box;-webkit-user-select:text;user-select:text;">正在读取诊断信息…</textarea>' +
        '<div style="display:flex;align-items:center;gap:8px;justify-content:flex-end;">' +
          '<button type="button" class="btn-text" data-role="refresh">刷新</button>' +
          '<button type="button" class="btn-tonal" data-role="copy">复制全部</button>' +
          '<button type="button" class="btn-filled" data-role="close">关闭</button>' +
        '</div>' +
      '</div>';

    var box = overlay.querySelector('[data-role="diag-box"]');

    function loadDiag() {
      if (box) box.value = '正在读取诊断信息…';
      App.utils.call('get_audio_diagnostics').then(function (text) {
        if (box) box.value = (typeof text === 'string' && text) ? text : String(text || '无返回数据');
      }).catch(function (err) {
        if (box) box.value = '读取失败: ' + (err && err.message ? err.message : String(err));
      });
    }

    function close() {
      if (overlay.parentNode) overlay.parentNode.removeChild(overlay);
    }

    var refreshBtn = overlay.querySelector('[data-role="refresh"]');
    var copyBtn = overlay.querySelector('[data-role="copy"]');
    var closeBtn = overlay.querySelector('[data-role="close"]');
    if (refreshBtn) refreshBtn.addEventListener('click', loadDiag);
    if (closeBtn) closeBtn.addEventListener('click', close);
    if (copyBtn) {
      copyBtn.addEventListener('click', function () {
        var content = box ? box.value : '';
        if (!content) return;
        function notifyOk() {
          if (App.utils && App.utils.toast) App.utils.toast('诊断信息已复制');
        }
        if (navigator.clipboard && navigator.clipboard.writeText) {
          navigator.clipboard.writeText(content).then(notifyOk).catch(function () {
            if (box) { box.focus(); box.select(); try { document.execCommand('copy'); } catch (_) {} }
            notifyOk();
          });
        } else {
          if (box) { box.focus(); box.select(); try { document.execCommand('copy'); } catch (_) {} }
          notifyOk();
        }
      });
    }

    overlay.addEventListener('click', function (e) {
      if (e.target === overlay) close();
    });

    document.body.appendChild(overlay);
    loadDiag();
  }

  window.__iosBindSettingsSection = function (container, section, settings) {
    if (!container || !section) return;
    if (settings) {
      if (Array.isArray(settings.ios_nav_items)) {
        _navItems = _normalizeNavItems(settings.ios_nav_items);
      }
      if (settings.ios_nav_more_style) {
        _moreStyle = settings.ios_nav_more_style === 'page' ? 'page' : 'popup';
      }
    }
    if (section.id === 'appearance') {
      _refreshIosNavCard(container);
    }
  };

  // ── 2. Bottom Navigation & Customizable "More" ───────────────────────

  function _closeMorePopup() {
    if (_morePopupEl) _morePopupEl.hidden = true;
    var moreBtn = document.getElementById('nav-more');
    if (moreBtn) moreBtn.setAttribute('aria-expanded', 'false');
  }

  function _navigateToNavItem(id) {
    _closeMorePopup();
    if (id === 'playlists') {
      var plBtn = document.getElementById('nav-playlists');
      if (plBtn) {
        plBtn.click();
        return;
      }
    }
    if (id === 'settings') {
      if (window.App && typeof App.navigate === 'function') {
        App.navigate('settings');
      } else {
        var topSettings = document.getElementById('title-bar-settings');
        if (topSettings) topSettings.click();
      }
      _syncNavActiveState('settings');
      return;
    }
    if (window.App && typeof App.navigate === 'function') {
      App.navigate(id);
      _syncNavActiveState(id);
    } else {
      var btn = NAV_CATALOG.find(function (c) { return c.id === id; });
      var el = btn && document.getElementById(btn.btnId);
      if (el) el.click();
    }
  }

  function _syncNavActiveState(pageId) {
    var navMain = document.querySelector('#nav-drawer .nav-main');
    if (!navMain) return;
    var inBar = _navItems.indexOf(pageId) >= 0;
    navMain.querySelectorAll('.nav-item').forEach(function (b) {
      var dp = b.getAttribute('data-page');
      if (b.id === 'nav-more') {
        b.classList.toggle('active', !inBar && pageId !== '');
      } else if (dp) {
        b.classList.toggle('active', dp === pageId);
      }
    });
  }

  function _openMorePopup() {
    if (!_morePopupEl) {
      _morePopupEl = document.createElement('div');
      _morePopupEl.className = 'ios-more-popup mdx';
      _morePopupEl.id = 'ios-more-popup';
      _morePopupEl.hidden = true;
      document.body.appendChild(_morePopupEl);
      document.addEventListener('click', function (e) {
        if (!_morePopupEl || _morePopupEl.hidden) return;
        var moreBtn = document.getElementById('nav-more');
        if (_morePopupEl.contains(e.target) || (moreBtn && moreBtn.contains(e.target))) return;
        _closeMorePopup();
      });
    }

    var hidden = _getHiddenNavItems();
    _morePopupEl.innerHTML =
      '<div class="md-seglist">' +
      hidden.map(function (item) {
        return '' +
          '<button type="button" class="nav-submenu-item" data-ios-more-id="' + item.id + '">' +
            '<span class="material-symbols-rounded nav-submenu-item-icon">' + item.icon + '</span>' +
            '<span class="nav-submenu-item-name">' + _t(item.labelKey, item.fallback) + '</span>' +
          '</button>';
      }).join('') +
      '</div>';

    _morePopupEl.querySelectorAll('[data-ios-more-id]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        _navigateToNavItem(this.getAttribute('data-ios-more-id'));
      });
    });

    _morePopupEl.hidden = false;
    var moreBtn = document.getElementById('nav-more');
    if (moreBtn) moreBtn.setAttribute('aria-expanded', 'true');
  }

  function _openMorePage() {
    _closeMorePopup();
    var container = document.getElementById('page-container');
    if (!container) return;
    document.body.classList.remove('settings-active');
    var hidden = _getHiddenNavItems();
    container.innerHTML = '' +
      '<div class="ios-more-page">' +
        '<h1 class="ios-more-page-title">' + _t('nav.more', '更多') + '</h1>' +
        '<div class="ios-more-grid">' +
          hidden.map(function (item) {
            return '' +
              '<button type="button" class="ios-more-card" data-ios-more-id="' + item.id + '">' +
                '<span class="material-symbols-rounded">' + item.icon + '</span>' +
                '<span>' + _t(item.labelKey, item.fallback) + '</span>' +
              '</button>';
          }).join('') +
        '</div>' +
      '</div>';
    container.querySelectorAll('[data-ios-more-id]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        _navigateToNavItem(this.getAttribute('data-ios-more-id'));
      });
    });
    _syncNavActiveState('__more__');
  }

  function _applyBottomNavLayout() {
    var navMain = document.querySelector('#nav-drawer .nav-main');
    if (!navMain) return;
    var phone = _isPhone();
    var moreBtn = document.getElementById('nav-more');

    if (!phone) {
      // iPad keeps desktop side rail intact
      NAV_CATALOG.forEach(function (item) {
        var el = document.getElementById(item.btnId);
        if (!el) return;
        el.classList.remove('ios-nav-hidden');
      });
      if (moreBtn) moreBtn.classList.add('ios-nav-hidden');
      return;
    }

    var shownSet = {};
    _navItems.forEach(function (id) { shownSet[id] = 1; });

    // Reorder visible buttons in user-configured order, hide the rest into "More"
    _navItems.forEach(function (id) {
      var item = NAV_CATALOG.find(function (c) { return c.id === id; });
      var el = item && document.getElementById(item.btnId);
      if (el) {
        el.classList.remove('ios-nav-hidden');
        navMain.appendChild(el);
      }
    });

    NAV_CATALOG.forEach(function (item) {
      if (shownSet[item.id]) return;
      var el = document.getElementById(item.btnId);
      if (el) el.classList.add('ios-nav-hidden');
    });

    if (moreBtn) {
      var hasHidden = _getHiddenNavItems().length > 0;
      moreBtn.classList.toggle('ios-nav-hidden', !hasHidden);
      navMain.appendChild(moreBtn);
    }
  }

  // ── 2b. 2nd-Level Subpage Back Bar & 2-Level Settings (iPhone) ───────

  var _subpageTitleText = '';

  function _ensureSubpageTitleEl() {
    var left = document.querySelector('.title-bar-left');
    if (!left) return null;
    var el = document.getElementById('ios-subpage-title');
    if (!el) {
      el = document.createElement('span');
      el.id = 'ios-subpage-title';
      el.className = 'ios-subpage-title';
      left.appendChild(el);
    }
    return el;
  }

  function _setSubpageTitle(text) {
    _subpageTitleText = text || '';
    var el = _ensureSubpageTitleEl();
    if (el) el.textContent = _subpageTitleText;
  }

  function _detectSubpage() {
    if (!_isPhone()) return false;
    if (document.body.classList.contains('np-fullscreen')) return false;
    if (document.body.classList.contains('ios-settings-detail')) return true;

    var curPage = (window.App && App.state && App.state.currentPage) || '';
    var curParams = (window.App && App.state && App.state.currentPageParams) || null;
    if (curPage === 'search' || curPage === 'about') return true;
    if (curPage === 'playlists' && curParams && curParams.playlist_id) return true;
    if (curPage === 'your_mix' && curParams && curParams.mix_detail) return true;

    var container = document.getElementById('page-container');
    if (!container) return false;
    if (container.querySelector('.detail-header, .artist-detail-body, .ym-mix-detail-page, .search-page, .playlist-header')) {
      return true;
    }
    return false;
  }

  function _syncSubpageState() {
    var isSub = _detectSubpage();
    document.body.classList.toggle('ios-subpage', isSub);

    var backBtn = document.getElementById('title-bar-back');
    if (backBtn && isSub) {
      backBtn.disabled = false;
      backBtn.style.opacity = '1';
      backBtn.style.pointerEvents = 'auto';
    }

    if (!document.body.classList.contains('ios-settings-detail')) {
      var titleEl = _ensureSubpageTitleEl();
      if (titleEl) {
        if (!isSub) {
          titleEl.textContent = '';
        } else {
          var curPage = (window.App && App.state && App.state.currentPage) || '';
          if (curPage === 'search') {
            titleEl.textContent = _t('common.search', '搜索');
          } else {
            var h = document.querySelector('#page-container .detail-name, #page-container .ym-mix-detail-title, #page-container .playlist-title');
            titleEl.textContent = h ? (h.textContent || '').trim() : '';
          }
        }
      }
    }
  }

  function _enhanceSettingsTwoLevel() {
    var nav = document.getElementById('settings-nav');
    if (!nav || nav.dataset.iosTwoLevelBound === '1') return;
    nav.dataset.iosTwoLevelBound = '1';

    // Always start at Level 1 (Category List) when entering Settings on iPhone
    document.body.classList.remove('ios-settings-detail');
    _setSubpageTitle('');

    nav.querySelectorAll('.settings-nav-item').forEach(function (item) {
      if (!item.querySelector('.ios-settings-chevron')) {
        var chev = document.createElement('span');
        chev.className = 'material-symbols-rounded ios-settings-chevron';
        chev.textContent = 'chevron_right';
        item.appendChild(chev);
      }
      item.addEventListener('click', function () {
        if (!_isPhone()) return;
        var labelEl = this.querySelector('.settings-nav-label');
        _setSubpageTitle(labelEl ? (labelEl.textContent || '').trim() : '');
        document.body.classList.add('ios-settings-detail');
        var pane = document.getElementById('content-pane');
        if (pane) pane.scrollTop = 0;
        _syncSubpageState();
      });
    });

    _syncSubpageState();
  }

  function _exitSettingsDetail() {
    if (!document.body.classList.contains('ios-settings-detail')) return false;
    document.body.classList.remove('ios-settings-detail');
    _setSubpageTitle('');
    var pane = document.getElementById('content-pane');
    if (pane) pane.scrollTop = 0;
    _syncSubpageState();
    return true;
  }

  function _setupSubpageObserver() {
    _ensureSubpageTitleEl();

    // Intercept Back button on iPhone:
    // 1. Settings Level-2 -> Settings Level-1 list
    // 2. In-place detail views (album / artist / mix detail) -> back to their parent list
    var backBtn = document.getElementById('title-bar-back');
    if (backBtn) {
      backBtn.addEventListener('click', function (e) {
        if (!_isPhone()) return;
        if (_exitSettingsDetail()) {
          e.preventDefault();
          e.stopImmediatePropagation();
          return;
        }
        var curPage = (window.App && App.state && App.state.currentPage) || '';
        var c = document.getElementById('page-container');
        if (c && curPage === 'albums' && c.querySelector('.detail-header')) {
          e.preventDefault();
          e.stopImmediatePropagation();
          App.navigate('albums');
          return;
        }
        if (c && curPage === 'artists' && c.querySelector('.artist-detail-body')) {
          e.preventDefault();
          e.stopImmediatePropagation();
          App.navigate('artists');
          return;
        }
        if (c && curPage === 'your_mix' && c.querySelector('.ym-mix-detail-page')) {
          e.preventDefault();
          e.stopImmediatePropagation();
          App.navigate('your_mix');
          return;
        }
      }, true);
    }

    var container = document.getElementById('page-container');
    if (container && window.MutationObserver) {
      var mo = new MutationObserver(function () {
        if (!document.getElementById('settings-nav')) {
          document.body.classList.remove('ios-settings-detail');
        } else {
          _enhanceSettingsTwoLevel();
        }
        _syncSubpageState();
      });
      mo.observe(container, { childList: true, subtree: false });
    }
  }

  // ── 3. iPhone Playback Sheet: Mini Player <-> Fullscreen (No Sidebar) ─

  function _setPhoneNpMode(mode) {
    var pane = document.getElementById('now-playing-pane');
    if (!pane) return;
    pane.classList.remove('ios-queue-open');
    var qPanel = pane.querySelector('.np-panel--queue');
    if (qPanel) qPanel.classList.remove('active');
    var targetSeg = pane.querySelector('.np-content-seg[data-mode="' + mode + '"]');
    if (targetSeg) {
      targetSeg.click();
    } else {
      pane.classList.remove('np-content-solo', 'np-content-duo', 'np-content-lyrics');
      pane.classList.add('np-content-' + mode);
    }
  }

  function _refreshPhoneQueuePanel() {
    var list = document.getElementById('np-queue-list');
    var q = (window.App && App.state && Array.isArray(App.state.queue)) ? App.state.queue : [];
    var idx = (window.App && App.state && typeof App.state.currentQueueIndex === 'number')
      ? App.state.currentQueueIndex
      : 0;
    if (q.length > 0 && window.App && App.nowPlaying && typeof App.nowPlaying.updateQueue === 'function') {
      App.nowPlaying.updateQueue(q, idx);
    } else if (list && q.length === 0) {
      list.innerHTML =
        '<div class="ios-queue-empty">' +
          '<span class="material-symbols-rounded">queue_music</span>' +
          '<p>' + _t('np.queueEmpty', '播放队列为空') + '</p>' +
        '</div>';
    }
  }

  function _openPhoneNowPlaying() {
    var pane = document.getElementById('now-playing-pane');
    if (!pane) return;
    pane.classList.remove('collapsed', 'ios-queue-open');
    var qPanel = pane.querySelector('.np-panel--queue');
    if (qPanel) qPanel.classList.remove('active');
    pane.classList.add('fullscreen');
    document.body.classList.remove('player-collapsed');
    document.body.classList.add('np-fullscreen');
    _syncSubpageState();

    // Enforce 2-mode (solo or lyrics) on iPhone: if currently in duo, switch to solo
    if (pane.classList.contains('np-content-duo') ||
        (!pane.classList.contains('np-content-solo') && !pane.classList.contains('np-content-lyrics'))) {
      _setPhoneNpMode('solo');
    }
  }

  function _closePhoneNowPlaying() {
    var pane = document.getElementById('now-playing-pane');
    if (!pane) return;
    pane.classList.remove('fullscreen', 'ios-queue-open');
    var qPanel = pane.querySelector('.np-panel--queue');
    if (qPanel) qPanel.classList.remove('active');
    pane.classList.add('collapsed');
    document.body.classList.remove('np-fullscreen');
    document.body.classList.add('player-collapsed');
    _syncSubpageState();
  }

  function _setupPhonePlaybackBehavior() {
    if (_isPhone()) {
      _closePhoneNowPlaying();
    }

    // Tap Mini Player body -> open fullscreen Now Playing directly (no right sidebar)
    var mini = document.getElementById('mini-player');
    if (mini) {
      mini.addEventListener('click', function (e) {
        if (!_isPhone()) return;
        if (e.target.closest('button')) return;
        e.stopImmediatePropagation();
        _openPhoneNowPlaying();
      }, true);
    }

    // Tap top-left collapse button -> collapse directly back to Mini Player
    var collapseBtn = document.getElementById('btn-collapse');
    if (collapseBtn) {
      collapseBtn.addEventListener('click', function (e) {
        if (!_isPhone()) return;
        e.preventDefault();
        e.stopImmediatePropagation();
        _closePhoneNowPlaying();
      }, true);
    }

    // Top-right queue button on iPhone: Apple Music mutual-exclusion logic with lyrics
    // (closed -> open queue & deactivate lyrics; open -> close queue & return to solo cover)
    var queueBtn = document.getElementById('btn-ios-queue');
    if (queueBtn) {
      queueBtn.addEventListener('click', function (e) {
        if (!_isPhone()) return;
        e.preventDefault();
        var pane = document.getElementById('now-playing-pane');
        if (!pane) return;
        var qPanel = pane.querySelector('.np-panel--queue');
        if (pane.classList.contains('ios-queue-open')) {
          _setPhoneNpMode('solo');
        } else {
          pane.classList.add('ios-queue-open');
          if (qPanel) qPanel.classList.add('active');
          requestAnimationFrame(function () {
            _refreshPhoneQueuePanel();
          });
        }
      });
    }

    // Clicking solo or lyrics segment while in Queue closes Queue and switches cleanly
    document.querySelectorAll('.np-content-seg').forEach(function (seg) {
      seg.addEventListener('click', function () {
        var pane = document.getElementById('now-playing-pane');
        if (!pane) return;
        pane.classList.remove('ios-queue-open');
        var qPanel = pane.querySelector('.np-panel--queue');
        if (qPanel) qPanel.classList.remove('active');
      });
    });

    // Bottom nav Settings button & More button
    var navSettings = document.getElementById('nav-settings');
    if (navSettings) {
      navSettings.addEventListener('click', function () {
        document.body.classList.remove('ios-settings-detail');
        _navigateToNavItem('settings');
      });
    }

    // Any non-More bottom nav click closes More popup, exits Settings detail & clears More highlight
    var navDrawer = document.getElementById('nav-drawer');
    if (navDrawer) {
      navDrawer.addEventListener('click', function (e) {
        var btn = e.target.closest('.nav-item');
        if (!btn || btn.id === 'nav-more') return;
        _closeMorePopup();
        document.body.classList.remove('ios-settings-detail');
        var moreEl = document.getElementById('nav-more');
        if (moreEl) moreEl.classList.remove('active');
      });
    }

    var navMore = document.getElementById('nav-more');
    if (navMore) {
      navMore.addEventListener('click', function (e) {
        e.stopPropagation();
        if (_moreStyle === 'page') {
          _openMorePage();
        } else {
          if (_morePopupEl && !_morePopupEl.hidden) {
            _closeMorePopup();
          } else {
            _openMorePopup();
          }
        }
      });
    }
  }

  // ── 4. Touch Ergonomics: No Zoom, Long-press Context Menu & Left-edge Swipe Back ─

  function _setupTouchGestures() {
    // Strictly prevent pinch-to-zoom & double-tap zoom gestures on iOS WKWebView
    ['gesturestart', 'gesturechange', 'gestureend'].forEach(function (evName) {
      document.addEventListener(evName, function (e) {
        e.preventDefault();
      }, { passive: false });
    });

    document.addEventListener('touchmove', function (e) {
      if (e.touches && e.touches.length > 1) {
        e.preventDefault();
      }
    }, { passive: false });

    // Long-press (500ms) -> synthetic contextmenu for track/album/artist rows
    var lpTimer = null;
    var lpStartX = 0;
    var lpStartY = 0;

    document.addEventListener('touchstart', function (e) {
      if (e.touches.length !== 1) return;
      var t = e.touches[0];
      lpStartX = t.clientX;
      lpStartY = t.clientY;
      var target = e.target.closest('.track-row, .album-card, .artist-row, .np-queue-item');
      if (!target) return;
      lpTimer = setTimeout(function () {
        lpTimer = null;
        var ev = new MouseEvent('contextmenu', {
          bubbles: true,
          cancelable: true,
          clientX: lpStartX,
          clientY: lpStartY,
          button: 2,
        });
        target.dispatchEvent(ev);
      }, 500);
    }, { passive: true });

    document.addEventListener('touchmove', function (e) {
      if (!lpTimer || e.touches.length !== 1) return;
      var t = e.touches[0];
      if (Math.hypot(t.clientX - lpStartX, t.clientY - lpStartY) > 12) {
        clearTimeout(lpTimer);
        lpTimer = null;
      }
    }, { passive: true });

    ['touchend', 'touchcancel'].forEach(function (name) {
      document.addEventListener(name, function () {
        if (lpTimer) {
          clearTimeout(lpTimer);
          lpTimer = null;
        }
      }, { passive: true });
    });

    // Left-edge swipe right -> Back button on subpages / Settings Level-2 / Now Playing
    var edgeActive = false;
    var edgeStartX = 0;
    var edgeStartY = 0;

    document.addEventListener('touchstart', function (e) {
      if (!_isPhone() || e.touches.length !== 1) return;
      var t = e.touches[0];
      if (t.clientX <= 24) {
        edgeActive = true;
        edgeStartX = t.clientX;
        edgeStartY = t.clientY;
      } else {
        edgeActive = false;
      }
    }, { passive: true });

    document.addEventListener('touchend', function (e) {
      if (!edgeActive) return;
      edgeActive = false;
      var t = e.changedTouches && e.changedTouches[0];
      if (!t) return;
      var dx = t.clientX - edgeStartX;
      var dy = Math.abs(t.clientY - edgeStartY);
      if (dx > 70 && dy < 60) {
        if (document.body.classList.contains('np-fullscreen')) {
          _closePhoneNowPlaying();
        } else if (!_exitSettingsDetail() && document.body.classList.contains('ios-subpage')) {
          var backBtn = document.getElementById('title-bar-back');
          if (backBtn) backBtn.click();
        }
      }
    }, { passive: true });
  }

  // ── 5. Bootstrap & Settings Sync ─────────────────────────────────────

  function _loadNavPrefs() {
    if (!window.App || !App.utils || !App.utils.call) return;
    App.utils.call('get_settings').then(function (raw) {
      try {
        var s = JSON.parse(raw);
        if (s && Array.isArray(s.ios_nav_items)) {
          _navItems = _normalizeNavItems(s.ios_nav_items);
        }
        if (s && s.ios_nav_more_style) {
          _moreStyle = s.ios_nav_more_style === 'page' ? 'page' : 'popup';
        }
        _applyBottomNavLayout();
      } catch (e) {}
    }).catch(function () {});
  }

  function _init() {
    _setupSubpageObserver();
    _setupPhonePlaybackBehavior();
    _setupTouchGestures();
    _applyBottomNavLayout();
    _syncSubpageState();
    window.addEventListener('resize', function () {
      _applyBottomNavLayout();
      _syncSubpageState();
    });

    if (window.__bridge && window.__bridge.on) {
      window.__bridge.on('settings_changed', function (raw) {
        try {
          var s = typeof raw === 'string' ? JSON.parse(raw) : raw;
          if (s && Array.isArray(s.ios_nav_items)) {
            _navItems = _normalizeNavItems(s.ios_nav_items);
          }
          if (s && s.ios_nav_more_style) {
            _moreStyle = s.ios_nav_more_style === 'page' ? 'page' : 'popup';
          }
          _applyBottomNavLayout();
        } catch (e) {}
      });
    }

    setTimeout(_loadNavPrefs, 250);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', _init);
  } else {
    _init();
  }
})();
