from flask import Flask, jsonify

app = Flask(__name__)

@app.route('/balance')
def get_balance():
    return jsonify({"balance": 100.00})

if __name__ == '__main__':
    app.run(debug=False, host='0.0.0.0', port=8080)