import java.io.File;
import java.io.StringReader;
import javax.xml.XMLConstants;
import javax.xml.transform.stream.StreamSource;
import javax.xml.validation.Schema;
import javax.xml.validation.SchemaFactory;
import javax.xml.validation.Validator;
import org.w3c.dom.ls.LSInput;
import org.w3c.dom.ls.LSResourceResolver;

/**
 * Validates XML documents against an XSD 1.0 schema with no network access.
 * Usage: java Validate.java ROOT_SCHEMA SCHEMA_DIR FILE...
 * Absolute http(s) schema locations (after resolving a relative one against its base) are mapped into SCHEMA_DIR: "xlink.xsd" and "xml.xsd" to the same names in SCHEMA_DIR,
 * and http://schemas.opengis.net/PATH to SCHEMA_DIR/PATH. Prints "OK file" or "FAIL file: message".
 * Exit status 0 when every file validated.
 */
public class Validate {
    public static void main(String[] args) throws Exception {
        File dir = new File(args[1]);
        SchemaFactory f = SchemaFactory.newInstance(XMLConstants.W3C_XML_SCHEMA_NS_URI);
        f.setResourceResolver(new LSResourceResolver() {
            public LSInput resolveResource(String type, String ns, String pub, String sys, String base) {
                if (sys == null) return null;
                try {
                    if (base != null) sys = new java.net.URI(base).resolve(sys).toString();
                } catch (Exception e) { return null; }
                if (!sys.startsWith("http")) return null;
                File local = sys.endsWith("/xlink.xsd") ? new File(dir, "xlink.xsd")
                    : sys.endsWith("/xml.xsd") ? new File(dir, "xml.xsd")
                    : new File(dir, sys.replaceFirst("^https?://schemas\\.opengis\\.net/", ""));
                if (!local.isFile()) return null;
                return new Input(local, sys);
            }
        });
        Schema schema = f.newSchema(new File(args[0]));
        int bad = 0;
        for (int i = 2; i < args.length; i++) {
            Validator v = schema.newValidator();
            try {
                v.validate(new StreamSource(new File(args[i])));
                System.out.println("OK " + args[i]);
            } catch (Exception e) {
                bad++;
                System.out.println("FAIL " + args[i] + ": " + String.valueOf(e.getMessage()).replace('\n', ' '));
            }
        }
        System.exit(bad == 0 ? 0 : 1);
    }

    static class Input implements LSInput {
        final File file; final String sys;
        Input(File file, String sys) { this.file = file; this.sys = sys; }
        public java.io.InputStream getByteStream() {
            try { return new java.io.FileInputStream(file); } catch (Exception e) { return null; }
        }
        public String getSystemId() { return sys; }
        public java.io.Reader getCharacterStream() { return null; }
        public void setCharacterStream(java.io.Reader r) {}
        public void setByteStream(java.io.InputStream s) {}
        public String getStringData() { return null; }
        public void setStringData(String s) {}
        public void setSystemId(String s) {}
        public String getPublicId() { return null; }
        public void setPublicId(String s) {}
        public String getBaseURI() { return null; }
        public void setBaseURI(String s) {}
        public String getEncoding() { return null; }
        public void setEncoding(String s) {}
        public boolean getCertifiedText() { return false; }
        public void setCertifiedText(boolean b) {}
    }
}
