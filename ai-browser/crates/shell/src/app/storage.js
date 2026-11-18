(async (args, expectedUrl) => {
  let db;
  let transaction;
  let timer;
  const owns = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
  const checkPage = () => {
    if (location.href !== expectedUrl) throw new Error('The target page changed; read storage again.');
  };
  const jsonValue = value => {
    if (value === undefined) return null;
    const visit = (item, seen = new Set()) => {
      if (item === null || typeof item === 'string' || typeof item === 'boolean') return;
      if (typeof item === 'number' && Number.isFinite(item)) return;
      if (typeof item !== 'object' || seen.has(item) ||
          (!Array.isArray(item) && Object.getPrototypeOf(item) !== Object.prototype)) {
        throw new Error('This record contains non-JSON data; inspect or edit it with F12 instead.');
      }
      seen.add(item);
      for (const child of Object.values(item)) visit(child, seen);
      seen.delete(item);
    };
    visit(value);
    return value;
  };
  const finish = value => {
    const result = { origin: location.origin, storage: args.storage, ...value };
    if (JSON.stringify(result).length > 128000) {
      throw new Error('Storage result is too large; request an exact key or a smaller limit, or use F12.');
    }
    return result;
  };
  const request = req => new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(new Error(req.error?.message || 'IndexedDB request failed.'));
  });
  try {
    checkPage();
    const write = args.action === 'set' || args.action === 'delete';
    const limit = args.limit ?? 50;
    const offset = args.offset ?? 0;
    if (args.storage !== 'indexeddb') {
      const storage = args.storage === 'local' ? localStorage : sessionStorage;
      if (write) {
        const before = storage.getItem(args.key);
        const after = args.action === 'delete' ? null : args.value;
        // Check the result size before committing, not after changing the value.
        const result = finish({ key: args.key, before, after, verified: true });
        checkPage();
        if (args.action === 'delete') storage.removeItem(args.key);
        else storage.setItem(args.key, args.value);
        if (storage.getItem(args.key) !== after) throw new Error('The stored value did not match; read it again before retrying.');
        return result;
      }
      const keys = owns(args, 'key') ? [args.key].filter(key => storage.getItem(key) !== null) :
        Array.from({ length: storage.length }, (_, i) => storage.key(i)).filter(key => key !== null);
      const matches = keys.filter(key => !args.key_contains || key.toLowerCase().includes(args.key_contains.toLowerCase()));
      return finish({ entries: matches.slice(offset, offset + limit).map(key => ({ key, value: storage.getItem(key) })),
        offset, has_more: matches.length > offset + limit, total: matches.length });
    }
    if (!args.database) {
      return finish({ databases: (await indexedDB.databases()).map(({ name, version }) => ({ name, version })) });
    }
    const open = indexedDB.open(args.database);
    db = await new Promise((resolve, reject) => {
      let expired = false;
      timer = setTimeout(() => { expired = true; reject(new Error('Opening IndexedDB timed out.')); }, 5000);
      open.onupgradeneeded = () => open.transaction.abort();
      open.onerror = () => { clearTimeout(timer); reject(new Error('Database does not exist or cannot be opened; list databases first.')); };
      open.onsuccess = () => { clearTimeout(timer); if (expired) open.result.close(); else resolve(open.result); };
    });
    db.onversionchange = () => db.close();
    if (!args.object_store) {
      const stores = Array.from(db.objectStoreNames);
      const object_stores = stores.length ? stores.map(name => {
        const store = db.transaction(name, 'readonly').objectStore(name);
        return { name, key_path: store.keyPath, auto_increment: store.autoIncrement,
          indexes: Array.from(store.indexNames) };
      }) : [];
      return finish({ database: args.database, version: db.version, object_stores });
    }
    checkPage();
    transaction = db.transaction(args.object_store, write ? 'readwrite' : 'readonly');
    const committed = new Promise((resolve, reject) => {
      transaction.oncomplete = resolve;
      transaction.onabort = () => reject(new Error(transaction.error?.message || 'IndexedDB transaction aborted.'));
      transaction.onerror = () => {};
    });
    // A request error can arrive before code awaits the commit promise.
    committed.catch(() => {});
    timer = setTimeout(() => { try { transaction.abort(); } catch {} }, 5000);
    const store = transaction.objectStore(args.object_store);
    const metadata = { database: args.database, object_store: args.object_store };
    if (write) {
      let key = args.key;
      if (store.keyPath !== null && args.action === 'set') {
        const atPath = path => path === '' ? args.value : path.split('.').reduce((value, part) => value?.[part], args.value);
        const inline = Array.isArray(store.keyPath) ? store.keyPath.map(atPath) : atPath(store.keyPath);
        if (inline === undefined || (Array.isArray(inline) && inline.some(value => value === undefined))) {
          throw new Error('The full record must include its existing inline key; generated keys are not supported.');
        }
        if (owns(args, 'key') && indexedDB.cmp(inline, key) !== 0) throw new Error('The inline key does not match the requested key.');
        key = inline;
      }
      if (key === undefined) throw new Error('An existing record key is required.');
      const before = jsonValue(await request(store.get(key)));
      const after = args.action === 'delete' ? null : args.value;
      finish({ ...metadata, key, before, after });
      checkPage();
      if (args.action === 'delete') await request(store.delete(key));
      else if (store.keyPath !== null) await request(store.put(args.value));
      else await request(store.put(args.value, key));
      const actual = jsonValue(await request(store.get(key)));
      if (JSON.stringify(actual) !== JSON.stringify(after)) throw new Error('The stored record did not match; read it again before retrying.');
      await committed;
      return finish({ ...metadata, key, before, after: actual, verified: true });
    }
    const entries = [];
    let skipped = 0;
    let has_more = false;
    await new Promise((resolve, reject) => {
      const cursor = store.openCursor(owns(args, 'key') ? IDBKeyRange.only(args.key) : null);
      cursor.onerror = () => reject(new Error(cursor.error?.message || 'Could not read records.'));
      cursor.onsuccess = () => {
        try {
          const item = cursor.result;
          if (!item) return resolve();
          const matches = !args.key_contains || JSON.stringify(item.key).toLowerCase().includes(args.key_contains.toLowerCase());
          if (matches && skipped++ >= offset) {
            if (entries.length >= limit) { has_more = true; return resolve(); }
            entries.push({ key: jsonValue(item.key), value: jsonValue(item.value) });
            finish({ ...metadata, entries });
          }
          item.continue();
        } catch (error) { reject(error); }
      };
    });
    await committed;
    return finish({ ...metadata, entries, offset, has_more });
  } catch (error) {
    try { transaction?.abort(); } catch {}
    return { error: error?.message || String(error) };
  } finally {
    clearTimeout(timer);
    db?.close();
  }
})
