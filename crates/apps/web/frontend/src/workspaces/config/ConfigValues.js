import { editableGroups } from './fields';

const catalog = editableGroups.flatMap(group => group.fields);
const integerKeys = new Set(catalog.filter(field => field.type === 'integer').map(field => field.key));
const percentKeys = new Set(catalog.filter(field => field.type === 'percent').map(field => field.key));

export class ConfigValues {
  static assertCatalog(fields, t) {
    if (!fields || typeof fields !== 'object' || Array.isArray(fields)) {
      throw new Error(t('config.catalogMissing'));
    }
    const keys = new Set();
    for (const field of catalog) {
      if (keys.has(field.key)) throw new Error(t('config.catalogDuplicate', { field: field.key }));
      keys.add(field.key);
    }
    const missing = [...keys].filter(key => !Object.hasOwn(fields, key));
    const unexpected = Object.keys(fields).filter(key => !keys.has(key));
    if (missing.length || unexpected.length) {
      throw new Error(t('config.catalogMismatch', { missing: missing.join(', '), unexpected: unexpected.join(', ') }));
    }
    for (const field of catalog) {
      const value = fields[field.key];
      const valid = field.type === 'boolean' ? typeof value === 'boolean'
        : field.type === 'tags' ? Array.isArray(value) && value.every(tag => typeof tag === 'string')
        : field.type === 'integer' ? typeof value === 'string' && /^-?\d+$/.test(value)
        : typeof value === 'string';
      if (!valid) throw new Error(t('config.catalogType', { field: field.key }));
    }
  }

  static clone(fields) {
    return Object.fromEntries(Object.entries(fields).map(([key, value]) => [key, Array.isArray(value) ? [...value] : value]));
  }

  static forEditing(fields) {
    const values = this.clone(fields);
    for (const key of percentKeys) {
      if (values[key] !== 'disabled') values[key] = this.percent(values[key]);
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

  static serialize(fields, t) {
    const entries = Object.entries(fields).map(([key, value]) => {
      if (!integerKeys.has(key)) {
        const wireValue = percentKeys.has(key) && value !== 'disabled' ? this.fromPercent(value) : value;
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
