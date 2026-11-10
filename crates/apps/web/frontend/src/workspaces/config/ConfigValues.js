import { ConfigFieldCatalog } from './fields';

export class ConfigValues {
  static assertCatalog(fields, kinds, t) {
    if (!fields || typeof fields !== 'object' || Array.isArray(fields)
      || !kinds || typeof kinds !== 'object' || Array.isArray(kinds)) {
      throw new Error(t('config.catalogMissing'));
    }
    const missing = Object.keys(kinds).filter(key => !Object.hasOwn(fields, key));
    const unexpected = Object.keys(fields).filter(key => !Object.hasOwn(kinds, key));
    if (missing.length || unexpected.length) {
      throw new Error(t('config.catalogMismatch', { missing: missing.join(', '), unexpected: unexpected.join(', ') }));
    }
    for (const [key, kind] of Object.entries(kinds)) {
      const value = fields[key];
      const valid = kind === 'boolean' ? typeof value === 'boolean'
        : kind === 'string_array' ? Array.isArray(value) && value.every(item => typeof item === 'string')
        : kind === 'integer' ? typeof value === 'string' && /^-?\d+$/.test(value)
        : (kind === 'text' || kind === 'toml') && typeof value === 'string';
      if (!valid) throw new Error(t('config.catalogType', { field: key }));
    }
  }

  static clone(fields) {
    return Object.fromEntries(Object.entries(fields).map(([key, value]) => [key, Array.isArray(value) ? [...value] : value]));
  }

  static forEditing(fields, kinds) {
    const values = this.clone(fields);
    for (const key of Object.keys(values)) {
      if (ConfigFieldCatalog.field(key, kinds[key]).type === 'percent' && values[key] !== 'disabled') {
        values[key] = this.percent(values[key]);
      }
    }
    return values;
  }

  static equal(value, baseline) {
    return Array.isArray(value)
      ? Array.isArray(baseline) && value.length === baseline.length && value.every((item, index) => item === baseline[index])
      : value === baseline;
  }

  static changes(fields, baseline) {
    return fields ? Object.fromEntries(Object.entries(fields).filter(([key, value]) => !this.equal(value, baseline[key]))) : {};
  }

  static serialize(fields, kinds, t) {
    const entries = Object.entries(fields).map(([key, value]) => {
      if (kinds[key] !== 'integer') {
        const wireValue = ConfigFieldCatalog.field(key, kinds[key]).type === 'percent' && value !== 'disabled'
          ? this.fromPercent(value) : value;
        return `${JSON.stringify(key)}:${JSON.stringify(wireValue)}`;
      }
      if (!/^-?\d+$/.test(value)) throw new Error(t('config.invalidInteger', { field: key }));
      return `${JSON.stringify(key)}:${BigInt(value).toString()}`;
    });
    return `{${entries.join(',')}}`;
  }

  static percent(value) {
    if (value === 'disabled') return '';
    if (!/^\d+$/.test(value)) return value;
    const integer = BigInt(value);
    const fraction = (integer % 1000n).toString().padStart(3, '0').replace(/0+$/, '');
    return `${integer / 1000n}${fraction ? `.${fraction}` : ''}`;
  }

  static fromPercent(value) {
    if (!/^(?:\d+(?:\.\d{0,3})?|\.\d{1,3})$/.test(value)) return value;
    const [integer, fraction = ''] = value.split('.');
    return (BigInt(integer || '0') * 1000n + BigInt(fraction.padEnd(3, '0'))).toString();
  }
}
